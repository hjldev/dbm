import { expect, test, type Page } from '@playwright/test'

async function mockedNative(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (data: unknown) => void>()
    const calls: { name: string; args: Record<string, any> }[] = []
    ;(window as any).isTauri = true
    ;(window as any).__DBM_CALLS__ = calls
    const columns = [{ name: 'id', dataType: 'BIGINT', nullable: false, binary: false }, { name: 'name', dataType: 'VARCHAR', nullable: true, binary: false }]
    const rows = Array.from({ length: 200 }, (_, index) => [{ kind: 'integer', value: String(index + 1) }, { kind: 'text', value: `用户 ${index + 1}` }])
    const profile = { id: 'local', name: '本地开发', host: '127.0.0.1', port: 3306, username: 'root', database: 'app', tls: false, caPem: null, caName: null, rememberPassword: true }
    ;(window as any).__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' } }, transformCallback: (callback: (data: unknown) => void) => { const id = callbacks.size + 1; callbacks.set(id, callback); return id }, unregisterCallback: (id: number) => callbacks.delete(id),
      invoke: async (name: string, args: Record<string, any>) => {
        calls.push({ name, args })
        if (name.startsWith('plugin:event|')) return 1
        switch (name) {
          case 'bootstrap': return { connections: [profile], settings: { theme: 'light', previewLimit: 1000, sidebarWidth: 250 }, configPath: '/Users/test/Library/Application Support/dev.dbm.desktop/settings.json' }
          case 'connect': return 'MySQL 8.4.0'
          case 'list_databases': return ['app']
          case 'list_objects': return [{ name: 'users', kind: 'table', engine: 'InnoDB', estimatedRows: '1000', comment: '用户表' }]
          case 'describe_table': return { fields: columns.map(column => ({ ...column, defaultValue: null, extra: column.name === 'id' ? 'auto_increment' : '', comment: '' })), indexes: [{ name: 'PRIMARY', unique: true, columns: ['id'] }], foreignKeys: [], engine: 'InnoDB', keyColumns: ['id'], editable: true, createSql: 'CREATE TABLE `users` (`id` BIGINT PRIMARY KEY, `name` VARCHAR(255)) ENGINE=InnoDB;' }
          case 'read_page': return { result: { columns, rows, totalRows: 200, truncated: false }, hasNext: args.request.page === 0 }
          case 'apply_changes': return args.request.changes.length
          case 'open_session': return 'session'
          case 'statement_at_cursor': return args.source
          case 'execute_script': {
            args.events.onmessage({ type: 'progress', index: 0, line: 1, sql: args.source })
            args.events.onmessage({ type: 'statement', outcome: { index: 0, sql: args.source, line: 1, elapsedMs: 2, affectedRows: '0', warnings: 0, lastInsertId: null, error: null, results: [{ columns: [columns[0], columns[0]], rows: [[{ kind: 'integer', value: '18446744073709551615' }, { kind: 'integer', value: '2' }]], totalRows: 1, truncated: false }] } })
            return { executionId: args.executionId, completed: 1, cancelled: false, sessionValid: true, inTransaction: false, database: 'app', error: null }
          }
          case 'save_sql_file': return '/tmp/query.sql'
          case 'export_preview': return '/tmp/query.csv'
          case 'save_connection': return args.input.profile
          case 'test_connection': return 'MySQL 8.4.0'
          default: return undefined
        }
      },
    }
  })
}
async function openTable(page: Page) {
  await mockedNative(page); await page.goto('/')
  await page.getByRole('button', { name: '本地开发', exact: true }).click()
  await page.getByRole('button', { name: 'app', exact: true }).click()
  await page.getByRole('button', { name: 'users', exact: true }).click()
  await expect(page.getByRole('cell', { name: '用户 1', exact: true })).toBeVisible()
}

test('browser shell explains native requirement and connection form works', async ({ page }) => {
  await page.goto('/'); await expect(page.getByText('此页面是浏览器预览。', { exact: false })).toBeVisible()
  await page.getByRole('button', { name: '新建数据库连接' }).click()
  await page.getByLabel('连接名称').fill('测试连接')
  await page.getByLabel('启用 TLS 加密').check()
  await expect(page.getByRole('button', { name: '选择 PEM 文件' })).toBeVisible()
  await page.getByRole('button', { name: '测试连接', exact: true }).click()
  await expect(page.getByText('请在 DBM 桌面应用中使用数据库功能。', { exact: false })).toBeVisible()
})

test('new connections remember passwords by default and reconnect without asking', async ({ page }) => {
  await mockedNative(page); await page.goto('/')
  await page.getByRole('button', { name: '新建数据库连接' }).click()
  await expect(page.getByLabel('记住密码（保存在配置文件中）')).toBeChecked()
  await expect(page.getByText('密码以明文保存在本机 settings.json 中，下次连接自动使用。')).toBeVisible()
  await page.getByLabel('连接名称').fill('自动保存密码')
  await page.getByLabel('密码', { exact: true }).fill('public-ui-password-fixture')
  await page.getByRole('button', { name: '保存连接', exact: true }).click()
  const saved = await page.evaluate(() => (window as any).__DBM_CALLS__.find((call: any) => call.name === 'save_connection').args.input)
  expect(saved.profile.rememberPassword).toBe(true)
  expect(saved.password).toBe('public-ui-password-fixture')
  const branch = page.locator('.connection-branch').filter({ has: page.getByRole('button', { name: '自动保存密码', exact: true }) })
  await branch.getByRole('button', { name: '自动保存密码', exact: true }).click()
  await branch.getByTitle('断开连接').click()
  await branch.getByRole('button', { name: '自动保存密码', exact: true }).click()
  await expect(branch.getByTitle('断开连接')).toBeVisible()
  await expect(page.locator('.password-modal')).toHaveCount(0)
  const connections = await page.evaluate(() => (window as any).__DBM_CALLS__.filter((call: any) => call.name === 'connect'))
  expect(connections).toHaveLength(2)
  expect(connections.every((call: any) => call.args.password === null)).toBe(true)
})

test('editing preserves saved passwords and settings expose the configuration location', async ({ page }) => {
  await mockedNative(page); await page.goto('/')
  await page.getByRole('button', { name: '本地开发', exact: true }).hover()
  await page.getByTitle('编辑连接', { exact: true }).click()
  await expect(page.getByLabel('密码', { exact: true })).toHaveValue('')
  await expect(page.getByPlaceholder('留空保留已保存的密码')).toBeVisible()
  await page.getByLabel('连接名称').fill('保留密码')
  await page.getByRole('button', { name: '保存连接', exact: true }).click()
  const saved = await page.evaluate(() => (window as any).__DBM_CALLS__.find((call: any) => call.name === 'save_connection').args.input)
  expect(saved.password).toBeNull()
  expect(saved.profile.rememberPassword).toBe(true)
  await page.getByRole('button', { name: '界面设置', exact: true }).click()
  await expect(page.locator('.config-path')).toHaveText('/Users/test/Library/Application Support/dev.dbm.desktop/settings.json')
  await expect(page.getByText('记住的密码以明文保存，可在退出 DBM 后管理此文件。')).toBeVisible()
})

test('table virtualizes rows, coalesces edits and preserves changes across tabs', async ({ page }) => {
  await openTable(page)
  expect(await page.locator('.data-row').count()).toBeLessThan(70)
  await page.getByRole('cell', { name: '用户 1', exact: true }).dblclick()
  await page.locator('.cell-input').fill('修改后的中文')
  await page.getByRole('button', { name: '应用', exact: true }).click()
  await expect(page.getByRole('button', { name: '保存变更 (1)' })).toBeEnabled()
  await page.locator('.topbar').getByRole('button', { name: '新建查询', exact: true }).click()
  await page.getByRole('tab', { name: /users/ }).click()
  await expect(page.getByRole('cell', { name: '修改后的中文', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '保存变更 (1)' }).click()
  await expect(page.getByRole('button', { name: '保存变更', exact: true })).toBeDisabled()
  const edits = await page.evaluate(() => (window as any).__DBM_CALLS__.find((call: any) => call.name === 'apply_changes').args.request.changes)
  expect(edits).toHaveLength(1); expect(edits[0]).toMatchObject({ kind: 'update', values: [null, { kind: 'text', value: '修改后的中文' }] })
  await page.getByRole('button', { name: '下一页 ›' }).click()
  const lastPage = await page.evaluate(() => (window as any).__DBM_CALLS__.filter((call: any) => call.name === 'read_page').at(-1).args.request.page)
  expect(lastPage).toBe(1)
})

test('table protects dirty data on page changes and closing', async ({ page }) => {
  await openTable(page); await page.getByRole('button', { name: '新增行', exact: false }).click()
  await page.getByRole('button', { name: '下一页 ›' }).click()
  await expect(page.getByRole('alertdialog')).toBeVisible(); await page.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page.getByRole('button', { name: '保存变更 (1)' })).toBeEnabled()
  await page.getByRole('button', { name: '关闭 users' }).click(); await expect(page.getByRole('alertdialog')).toBeVisible()
  await page.getByRole('button', { name: '放弃变更', exact: true }).click(); await expect(page.getByRole('tab', { name: /users/ })).toHaveCount(0)
})

test('SQL editor runs native IPC, handles duplicate names and exact integers', async ({ page }) => {
  await mockedNative(page); await page.goto('/'); await page.getByRole('button', { name: '本地开发', exact: true }).click()
  await page.locator('.topbar').getByRole('button', { name: '新建查询', exact: true }).click()
  await page.locator('.cm-content').fill('SELECT 18446744073709551615 AS id, 2 AS id;')
  await page.getByRole('button', { name: '执行全部', exact: true }).click()
  await expect(page.getByRole('cell', { name: '18446744073709551615', exact: true })).toBeVisible()
  await expect(page.getByRole('columnheader', { name: 'id BIGINT' })).toHaveCount(2)
  await page.getByRole('button', { name: '保存 SQL', exact: true }).click(); await expect(page.getByRole('tab', { name: /query.sql/ })).toBeVisible()
  await page.getByRole('button', { name: '导出预览 CSV' }).click(); await expect(page.getByText('CSV 已导出：/tmp/query.csv')).toBeVisible()
  await page.getByRole('button', { name: '执行记录', exact: false }).click(); await expect(page.getByText('语句 1', { exact: true })).toBeVisible()
})

test('repeated query tabs release editor views, results and backend sessions', async ({ page }) => {
  await mockedNative(page); await page.goto('/')
  await page.getByRole('button', { name: '本地开发', exact: true }).click()
  for (let index = 0; index < 12; index++) {
    await page.locator('.topbar').getByRole('button', { name: '新建查询', exact: true }).click()
    await page.locator('.cm-content').fill('SELECT 1;')
    await page.getByRole('button', { name: '执行全部', exact: true }).click()
    await expect(page.getByRole('cell', { name: '18446744073709551615', exact: true })).toBeVisible()
    await page.getByRole('button', { name: `关闭 查询 ${index + 1}`, exact: true }).click()
    await page.getByRole('button', { name: '关闭查询', exact: true }).click()
    await expect(page.locator('.cm-editor')).toHaveCount(0)
    await expect(page.locator('.workspace-panel')).toHaveCount(0)
    await expect(page.getByRole('tab')).toHaveCount(0)
  }
  const counts = await page.evaluate(() => {
    const calls = (window as any).__DBM_CALLS__
    return { opened: calls.filter((call: any) => call.name === 'open_session').length, closed: calls.filter((call: any) => call.name === 'close_session').length }
  })
  expect(counts).toEqual({ opened: 12, closed: 12 })
})
