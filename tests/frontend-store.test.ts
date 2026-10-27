import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { MAX_OUTCOMES, useDbm } from '../src/store'
import type { ConnectionProfile, ExecutionEvent, SqlTab, StatementOutcome } from '../src/types'
const mock = vi.hoisted(() => ({ command: vi.fn() }))
vi.mock('../src/api', () => ({ command: mock.command, nativeAvailable: () => true, normalizeError: (error: unknown) => error, Channel: class { onmessage: (event: ExecutionEvent) => void = () => {} } }))
const profile = (id: string, database: string): ConnectionProfile => ({ id, database, name: id, host: '127.0.0.1', port: 3306, username: 'root', tls: false, caPem: null, caName: null, rememberPassword: false })
const outcome = (): StatementOutcome => ({ index: 0, line: 1, elapsedMs: 1, affectedRows: '0', warnings: 0, lastInsertId: null, sql: 'SELECT 1;', results: [{ columns: [{ name: '1', dataType: 'INT', nullable: false, binary: false }], rows: [[{ kind: 'integer', value: '1' }]], totalRows: 1, truncated: false }], error: null })
beforeEach(() => { setActivePinia(createPinia()); mock.command.mockReset() })
function setup() { const store = useDbm(); store.connections = [profile('a', 'database_a'), profile('b', 'database_b')]; store.connected = { a: '8.4', b: '8.4' }; return store }
describe('workspace session lifecycle', () => {
  it('does not carry the selected database across different connections', () => {
    const store = setup(); store.selectedConnection = 'a'; store.selectedDatabase = 'database_a'; store.newQuery('b')
    expect(store.activeTab?.database).toBe('database_b')
    store.newQuery('b', null); expect(store.activeTab?.database).toBeNull()
  })
  it('keeps a busy tab and refuses closing while execution is unfinished', async () => {
    const store = setup(); store.newQuery('a'); const tab = store.activeTab as SqlTab; tab.busy = true
    await store.closeTab(tab.id); expect(store.tabs).toHaveLength(1); expect(mock.command).not.toHaveBeenCalled()
  })
  it('preserves dirty table state when close confirmation is cancelled', async () => {
    const store = setup(); store.openTable('a', 'database_a', 'users'); const tab = store.activeTab!; if (tab.kind === 'table') tab.dirty = true
    const closing = store.closeTab(tab.id); expect(store.confirmDialog?.title).toContain('未保存'); store.answerConfirm(false); await closing
    expect(store.tabs).toHaveLength(1)
  })
  it('requires explicit rollback when the native session reports an open transaction', async () => {
    const store = setup(); store.newQuery('a'); const tab = store.activeTab as SqlTab; tab.sessionId = 'session'
    mock.command.mockRejectedValueOnce({ code: 'TRANSACTION_OPEN', message: 'open' }).mockResolvedValueOnce(undefined)
    const closing = store.closeTab(tab.id); await Promise.resolve(); await Promise.resolve()
    expect(store.confirmDialog?.action).toBe('回滚并关闭'); store.answerConfirm(true); await closing
    expect(mock.command).toHaveBeenLastCalledWith('close_session', { sessionId: 'session', rollback: true }); expect(store.tabs).toHaveLength(0)
  })
  it('invalidates a lost session and does not silently reconnect', async () => {
    const store = setup(); store.newQuery('a'); const tab = store.activeTab as SqlTab; tab.sessionId = 'lost'
    mock.command.mockRejectedValue({ code: 'SESSION_LOST', message: 'lost' }); await store.execute(tab, 'SELECT 1')
    expect(tab.sessionValid).toBe(false); expect(tab.busy).toBe(false); await expect(store.ensureSession(tab)).rejects.toMatchObject({ code: 'SESSION_LOST' })
  })
  it('retains at most ten previews and clearing history resets eviction bookkeeping', async () => {
    const store = setup(); store.newQuery('a'); const tab = store.activeTab as SqlTab; tab.sessionId = 'session'
    mock.command.mockImplementation(async (name: string, args: { events: { onmessage: (event: ExecutionEvent) => void } }) => {
      if (name === 'execute_script') { args.events.onmessage({ type: 'statement', outcome: outcome() }); return { completed: 1, cancelled: false, sessionValid: true, inTransaction: false, database: 'database_a', error: null } }
    })
    for (let index = 0; index < 12; index++) await store.execute(tab, 'SELECT 1')
    expect(tab.outcomes.slice(0, 2).every(item => item.results[0]!.rows.length === 0)).toBe(true)
    expect(tab.outcomes.slice(2).every(item => item.results[0]!.rows.length === 1)).toBe(true)
    store.clearResults(tab)
    for (let index = 0; index < 10; index++) await store.execute(tab, 'SELECT 1')
    expect(tab.outcomes.every(item => item.results[0]!.rows.length === 1)).toBe(true)
  })
  it('counts each result set when one statement returns more than ten sets', async () => {
    const store = setup(); store.newQuery('a'); const tab = store.activeTab as SqlTab; tab.sessionId = 'session'
    mock.command.mockImplementation(async (name: string, args: { events: { onmessage: (event: ExecutionEvent) => void } }) => {
      if (name === 'execute_script') {
        const statement = outcome(); statement.results = Array.from({ length: 12 }, () => outcome().results[0]!)
        args.events.onmessage({ type: 'statement', outcome: statement })
        return { completed: 1, cancelled: false, sessionValid: true, inTransaction: false, database: 'database_a', error: null }
      }
    })
    await store.execute(tab, 'CALL returns_many_sets()')
    expect(tab.outcomes[0]!.results.slice(0, 2).every(result => result.rows.length === 0)).toBe(true)
    expect(tab.outcomes[0]!.results.slice(2).every(result => result.rows.length === 1)).toBe(true)
  })
  it('applies execution events in batches and keeps the final status', async () => {
    vi.useFakeTimers()
    try {
      const store = setup(); store.newQuery('a'); const tab = store.activeTab as SqlTab; tab.sessionId = 'session'
      let events!: { onmessage: (event: ExecutionEvent) => void }
      let finish!: (report: unknown) => void
      mock.command.mockImplementation((name: string, args: { events: typeof events }) => {
        if (name === 'execute_script') { events = args.events; return new Promise(resolve => { finish = resolve }) }
      })
      const running = store.execute(tab, 'SELECT 1; SELECT 1')
      await vi.advanceTimersByTimeAsync(0)
      for (let index = 0; index < 2; index++) { events.onmessage({ type: 'progress', index, line: 1 }); events.onmessage({ type: 'statement', outcome: outcome() }) }
      expect(tab.outcomes).toHaveLength(0)
      await vi.advanceTimersByTimeAsync(50)
      expect(tab.outcomes).toHaveLength(2); expect(tab.selectedOutcome).toBe(1); expect(tab.progress).toContain('第 2 条')
      finish({ completed: 2, cancelled: false, sessionValid: true, inTransaction: false, database: 'database_a', error: null }); await running
      events.onmessage({ type: 'progress', index: 1, line: 1 }); await vi.advanceTimersByTimeAsync(50)
      expect(tab.progress).toBe('执行完成 · 2 条语句')
    } finally { vi.useRealTimers() }
  })
  it('keeps the newest log entries with absolute numbering and ten previews', async () => {
    const store = setup(); store.newQuery('a'); const tab = store.activeTab as SqlTab; tab.sessionId = 'session'
    mock.command.mockImplementation(async (name: string, args: { events: { onmessage: (event: ExecutionEvent) => void } }) => {
      if (name === 'execute_script') {
        for (let index = 0; index < MAX_OUTCOMES + 200; index++) args.events.onmessage({ type: 'statement', outcome: { ...outcome(), index } })
        return { completed: MAX_OUTCOMES + 200, cancelled: false, sessionValid: true, inTransaction: false, database: 'database_a', error: null }
      }
    })
    const previews = () => tab.outcomes.filter(item => item.results[0]!.rows.length).length
    await store.execute(tab, 'SELECT 1')
    expect(tab.outcomes).toHaveLength(MAX_OUTCOMES); expect(tab.outcomeOffset).toBe(200)
    expect(tab.outcomes[0]!.index).toBe(200); expect(tab.selectedOutcome).toBe(MAX_OUTCOMES - 1)
    expect(previews()).toBe(10); expect(tab.outcomes.slice(-10).every(item => item.results[0]!.rows.length === 1)).toBe(true)
    // The second run evicts previews whose log entries were already trimmed.
    await store.execute(tab, 'SELECT 1')
    expect(tab.outcomeOffset).toBe(2 * (MAX_OUTCOMES + 200) - MAX_OUTCOMES); expect(previews()).toBe(10)
    store.clearResults(tab)
    expect(tab.outcomes).toHaveLength(0); expect(tab.outcomeOffset).toBe(0)
  })
})
