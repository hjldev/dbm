import { expect, test, type Page } from '@playwright/test'

async function mockedNative(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (data: unknown) => void>()
    const calls: { name: string; args: Record<string, any> }[] = []
    ;(window as any).isTauri = true
    ;(window as any).__DBM_CALLS__ = calls
    const columns = [{ name: 'id', dataType: 'BIGINT', nullable: false, binary: false }, { name: 'name', dataType: 'VARCHAR', nullable: true, binary: false }]
    const rows: { kind: string; value?: string }[][] = Array.from({ length: 200 }, (_, index) => [{ kind: 'integer', value: String(index + 1) }, { kind: 'text', value: `用户 ${index + 1}` }])
    const fields = columns.map(column => ({ ...column, dataType: column.name === 'id' ? 'bigint' : 'varchar(255)', defaultValue: null, extra: column.name === 'id' ? 'auto_increment' : '', comment: '' }))
    const indexes = [{ name: 'PRIMARY', unique: true, columns: ['id'] }, { name: 'idx_name', unique: false, columns: ['name'] }]
    const definitions: Record<string, { name: string; definition: string; indexKind: string | null }> = {
      id: { name: 'id', definition: 'bigint NOT NULL AUTO_INCREMENT', indexKind: null },
      name: { name: 'name', definition: 'varchar(255) DEFAULT NULL', indexKind: null },
      PRIMARY: { name: 'PRIMARY', definition: '(`id`)', indexKind: 'primary' },
      idx_name: { name: 'idx_name', definition: "(`name`(20) DESC) COMMENT 'keep'", indexKind: 'index' },
    }
    let createSql = 'CREATE TABLE `users` (`id` BIGINT PRIMARY KEY, `name` VARCHAR(255)) ENGINE=InnoDB;'
    function schemaSql(request: any) {
      const { target, value } = request
      const action = target.action ?? 'modify'
      const indexSql = value.indexKind === 'primary' ? `PRIMARY KEY ${value.definition}` : `${value.indexKind === 'unique' ? 'UNIQUE ' : value.indexKind === 'fulltext' ? 'FULLTEXT ' : value.indexKind === 'spatial' ? 'SPATIAL ' : ''}INDEX \`${value.name}\` ${value.definition}`
      const dropSql = target.kind === 'field' ? `DROP COLUMN \`${target.originalName}\`` : target.originalName === 'PRIMARY' ? 'DROP PRIMARY KEY' : `DROP INDEX \`${target.originalName}\``
      const clause = action === 'drop' ? dropSql : action === 'add' ? target.kind === 'field' ? `ADD COLUMN \`${value.name}\` ${value.definition}` : `ADD ${indexSql}` : target.kind === 'field' ? `CHANGE COLUMN \`${target.originalName}\` \`${value.name}\` ${value.definition}` : `${dropSql}, ADD ${indexSql}`
      return `ALTER TABLE \`${target.database}\`.\`${target.table}\`\n  ${clause}`
    }
    function currentSchema(target: any) {
      if ((window as any).__DBM_SCHEMA_CONFLICT__ || target.expectedCreateSql !== createSql) throw { code: 'SCHEMA_CONFLICT', message: '表结构已改变，请刷新后重新修改。' }
    }
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
          case 'list_columns': return fields.map((field: { name: string }) => field.name)
          case 'describe_table': return { fields, indexes, foreignKeys: [], engine: (window as any).__DBM_VIEW__ ? null : 'InnoDB', keyColumns: ['id'], editable: true, createSql }
          case 'schema_definition': {
            currentSchema(args.target)
            if (args.target.action === 'add') return { name: '', definition: args.target.kind === 'field' ? 'varchar(255) DEFAULT NULL' : '(`id`)', indexKind: args.target.kind === 'field' ? null : 'index' }
            return { ...definitions[args.target.originalName] }
          }
          case 'preview_schema_change': {
            currentSchema(args.request.target)
            return schemaSql(args.request)
          }
          case 'apply_schema_change': {
            currentSchema(args.request.target)
            if ((window as any).__DBM_SCHEMA_FAILURE__) throw { code: 'MYSQL', message: 'Duplicate entry for key' }
            const { target, value } = args.request
            const action = target.action ?? 'modify'
            if (args.expectedSql !== schemaSql(args.request)) throw { code: 'SCHEMA_CONFLICT', message: 'SQL 预览已失效，请重新预览。' }
            if (action !== 'add') delete definitions[target.originalName]
            if (action !== 'drop') definitions[value.name] = { ...value }
            if (target.kind === 'field') {
              const position = fields.findIndex(field => field.name === target.originalName)
              if (action === 'add') {
                const column = { name: value.name, dataType: value.definition.split(' ')[0], nullable: !value.definition.includes('NOT NULL'), binary: false }
                fields.push({ ...column, defaultValue: null, extra: '', comment: '' }); columns.push(column)
                rows.forEach(row => row.push({ kind: 'null' }))
              } else if (action === 'drop') {
                fields.splice(position, 1); columns.splice(position, 1); rows.forEach(row => row.splice(position, 1))
                for (let index = indexes.length - 1; index >= 0; index--) {
                  indexes[index].columns = indexes[index].columns.filter((column: string) => column !== target.originalName)
                  if (!indexes[index].columns.length) { delete definitions[indexes[index].name]; indexes.splice(index, 1) }
                }
              } else {
                fields[position].name = value.name; fields[position].dataType = value.definition.split(' ')[0]
                columns[position].name = value.name
                indexes.forEach(index => { index.columns = index.columns.map(column => column === target.originalName ? value.name : column) })
              }
            } else {
              const position = indexes.findIndex(index => index.name === target.originalName)
              const index = { name: value.name, unique: ['primary', 'unique'].includes(value.indexKind), columns: Array.from(value.definition.matchAll(/`([^`]+)`/g), (match: any) => match[1]) as string[] }
              if (action === 'add') indexes.push(index)
              else if (action === 'drop') indexes.splice(position, 1)
              else indexes[position] = index
            }
            createSql = `CREATE TABLE users /* changed ${calls.length} */`
            return undefined
          }
          case 'read_page': return { result: { columns, rows, totalRows: 200, truncated: false }, hasNext: args.request.page === 0 }
          case 'apply_changes': return args.request.changes.length
          case 'open_session': return 'session'
          case 'statement_at_cursor': return args.source
          case 'execute_script': {
            args.events.onmessage({ type: 'progress', index: 0, line: 1 })
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

test('field edits require a current SQL preview and refresh data after saving', async ({ page }) => {
  await openTable(page)
  await page.getByRole('button', { name: '下一页 ›' }).click()
  await page.getByRole('button', { name: '字段', exact: true }).click()
  await page.getByRole('button', { name: '修改字段 name', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog.getByLabel('字段定义')).toHaveValue('varchar(255) DEFAULT NULL')
  await expect(dialog.getByRole('button', { name: '预览 SQL' })).toBeDisabled()
  await dialog.getByLabel('字段名称').fill('display_name')
  await dialog.getByLabel('字段定义').fill("varchar(300) NOT NULL DEFAULT '' COMMENT '显示名称'")
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await expect(dialog.locator('.schema-preview')).toContainText('CHANGE COLUMN `name` `display_name`')
  await dialog.getByLabel('字段定义').fill("varchar(400) NOT NULL DEFAULT '' COMMENT '显示名称'")
  await expect(dialog.locator('.schema-preview')).toHaveCount(0)
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await dialog.getByRole('button', { name: '执行修改' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('cell', { name: 'display_name', exact: true })).toBeVisible()
  const saved = await page.evaluate(() => (window as any).__DBM_CALLS__.find((call: any) => call.name === 'apply_schema_change'))
  expect(saved.args.request.value).toMatchObject({ name: 'display_name', definition: "varchar(400) NOT NULL DEFAULT '' COMMENT '显示名称'" })
  expect(saved.args.expectedSql).toContain('varchar(400)')
  await page.getByRole('button', { name: '数据', exact: true }).click()
  await expect(page.getByText('第 1 页', { exact: false })).toBeVisible()
  const lastRead = await page.evaluate(() => (window as any).__DBM_CALLS__.filter((call: any) => call.name === 'read_page').at(-1).args.request)
  expect(lastRead).toMatchObject({ page: 0, sortColumn: null, filters: [] })
})

test('fields can be added and deleted with metadata and data refreshed', async ({ page }) => {
  await openTable(page)
  await page.getByRole('button', { name: '下一页 ›' }).click()
  await page.getByRole('button', { name: '字段', exact: true }).click()
  await page.getByRole('button', { name: '新增字段', exact: false }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog.getByLabel('字段名称')).toHaveValue('')
  await expect(dialog.getByLabel('字段定义')).toHaveValue('varchar(255) DEFAULT NULL')
  await expect(dialog.getByRole('button', { name: '预览 SQL' })).toBeDisabled()
  await dialog.getByLabel('字段名称').fill('nickname')
  await dialog.getByLabel('字段定义').fill("varchar(80) DEFAULT NULL COMMENT '昵称'")
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await expect(dialog.locator('.schema-preview')).toContainText("ADD COLUMN `nickname` varchar(80) DEFAULT NULL COMMENT '昵称'")
  await page.screenshot({ path: 'test-results/schema-add-field.png' })
  await dialog.getByRole('button', { name: '执行新增' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('cell', { name: 'nickname', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '数据', exact: true }).click()
  await expect(page.getByRole('columnheader', { name: /nickname/ })).toBeVisible()
  await expect(page.getByText('第 1 页', { exact: false })).toBeVisible()
  await expect(page.getByRole('cell', { name: '用户 1', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '字段', exact: true }).click()
  await page.getByRole('button', { name: '删除字段 nickname', exact: true }).click()
  await expect(dialog).toContainText('nickname')
  await expect(dialog.getByLabel('字段名称')).toHaveCount(0)
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await expect(dialog.locator('.schema-preview')).toContainText('DROP COLUMN `nickname`')
  await expect(dialog.getByRole('button', { name: '执行删除' })).toBeDisabled()
  await page.screenshot({ path: 'test-results/schema-drop-field.png' })
  await dialog.getByLabel('我确认删除此字段及其数据').check()
  await dialog.getByRole('button', { name: '执行删除' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('cell', { name: 'nickname', exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: '数据', exact: true }).click()
  await expect(page.getByRole('columnheader', { name: /nickname/ })).toHaveCount(0)
  await expect(page.getByRole('cell', { name: '用户 1', exact: true })).toBeVisible()
  const saved = await page.evaluate(() => (window as any).__DBM_CALLS__.filter((call: any) => call.name === 'apply_schema_change'))
  expect(saved).toHaveLength(2)
  expect(saved[0].args.request.target).toMatchObject({ action: 'add', kind: 'field', originalName: '' })
  expect(saved[1].args.request.target).toMatchObject({ action: 'drop', kind: 'field', originalName: 'nickname' })
  expect(saved[1].args.request.target.expectedCreateSql).not.toBe(saved[0].args.request.target.expectedCreateSql)
})

test('indexes can be added and deleted, including primary keys', async ({ page }) => {
  await openTable(page)
  await page.getByRole('button', { name: '索引', exact: true }).click()
  await page.getByRole('button', { name: '新增索引', exact: false }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog.getByLabel('索引名称')).toHaveValue('')
  await expect(dialog.getByLabel('索引类型')).toHaveValue('index')
  await expect(dialog.getByLabel('索引定义')).toHaveValue('(`id`)')
  await dialog.getByLabel('索引名称').fill('uq_name')
  await dialog.getByLabel('索引类型').selectOption('unique')
  await dialog.getByLabel('索引定义').fill("(`name`(40) DESC) COMMENT '新增索引'")
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await expect(dialog.locator('.schema-preview')).toContainText("ADD UNIQUE INDEX `uq_name` (`name`(40) DESC) COMMENT '新增索引'")
  await dialog.getByRole('button', { name: '执行新增' }).click()
  await expect(dialog).toHaveCount(0)
  const indexRow = page.getByRole('row').filter({ has: page.getByRole('cell', { name: 'uq_name', exact: true }) })
  await expect(indexRow).toContainText('唯一索引')
  await expect(indexRow.getByRole('cell', { name: 'name', exact: true })).toBeVisible()
  for (const name of ['uq_name', 'PRIMARY']) {
    await page.getByRole('button', { name: `删除索引 ${name}`, exact: true }).click()
    await dialog.getByRole('button', { name: '预览 SQL' }).click()
    await expect(dialog.locator('.schema-preview')).toContainText(name === 'PRIMARY' ? 'DROP PRIMARY KEY' : 'DROP INDEX `uq_name`')
    await expect(dialog.getByRole('button', { name: '执行删除' })).toBeDisabled()
    await dialog.getByLabel('我确认删除此索引').check()
    await dialog.getByRole('button', { name: '执行删除' }).click()
    await expect(dialog).toHaveCount(0)
    await expect(page.getByRole('cell', { name, exact: true })).toHaveCount(0)
  }
  await expect(page.getByRole('cell', { name: 'idx_name', exact: true })).toBeVisible()
  const saved = await page.evaluate(() => (window as any).__DBM_CALLS__.filter((call: any) => call.name === 'apply_schema_change'))
  expect(saved.map((call: any) => call.args.request.target.action)).toEqual(['add', 'drop', 'drop'])
  expect(saved[2].args.expectedSql).toContain('DROP PRIMARY KEY')
})

test('deletion can be cancelled and requires acknowledgement for each preview', async ({ page }) => {
  await openTable(page)
  await page.getByRole('button', { name: '字段', exact: true }).click()
  await page.getByRole('button', { name: '删除字段 name', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: '取消', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('alertdialog')).toHaveCount(0)
  await expect(page.getByRole('cell', { name: 'name', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '删除字段 name', exact: true }).click()
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  const execute = dialog.getByRole('button', { name: '执行删除' })
  const acknowledge = dialog.getByLabel('我确认删除此字段及其数据')
  await expect(execute).toBeDisabled()
  await acknowledge.check()
  await expect(execute).toBeEnabled()
  await acknowledge.uncheck()
  await expect(execute).toBeDisabled()
  await dialog.getByRole('button', { name: '取消', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await page.getByRole('button', { name: '删除字段 name', exact: true }).click()
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await expect(acknowledge).not.toBeChecked()
  await expect(execute).toBeDisabled()
  await dialog.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page.getByRole('cell', { name: 'name', exact: true })).toBeVisible()
  const applied = await page.evaluate(() => (window as any).__DBM_CALLS__.filter((call: any) => call.name === 'apply_schema_change'))
  expect(applied).toHaveLength(0)
})

test('new field drafts survive cancelled dismissal and stale previews or execution', async ({ page }) => {
  await openTable(page)
  await page.getByRole('button', { name: '字段', exact: true }).click()
  await page.getByRole('button', { name: '新增字段', exact: false }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('字段名称').fill('draft_field')
  await dialog.getByLabel('字段定义').fill('int NOT NULL DEFAULT 0')
  await dialog.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page.getByRole('alertdialog')).toContainText('未保存的结构变更')
  await page.getByRole('alertdialog').getByRole('button', { name: '取消', exact: true }).click()
  await expect(dialog.getByLabel('字段名称')).toHaveValue('draft_field')
  await page.evaluate(() => { (window as any).__DBM_SCHEMA_CONFLICT__ = true })
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await expect(dialog.getByRole('alert')).toContainText('表结构已改变')
  await expect(dialog.getByRole('button', { name: '执行新增' })).toHaveCount(0)
  await expect(dialog.getByLabel('字段定义')).toHaveValue('int NOT NULL DEFAULT 0')
  await page.evaluate(() => { (window as any).__DBM_SCHEMA_CONFLICT__ = false })
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await page.evaluate(() => { (window as any).__DBM_SCHEMA_CONFLICT__ = true })
  await dialog.getByRole('button', { name: '执行新增' }).click()
  await expect(dialog.getByRole('alert')).toContainText('表结构已改变')
  await expect(dialog.locator('.schema-preview')).toHaveCount(0)
  await expect(dialog.getByLabel('字段名称')).toHaveValue('draft_field')
  await expect(dialog.getByLabel('字段定义')).toHaveValue('int NOT NULL DEFAULT 0')
  await expect(page.getByRole('cell', { name: 'draft_field', exact: true })).toHaveCount(0)
  await page.evaluate(() => { (window as any).__DBM_SCHEMA_CONFLICT__ = false })
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await dialog.getByRole('button', { name: '执行新增' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('cell', { name: 'draft_field', exact: true })).toBeVisible()
})

test('index editor preserves options, protects drafts and retains failed edits', async ({ page }) => {
  await openTable(page)
  await page.getByRole('button', { name: '索引', exact: true }).click()
  await page.getByRole('button', { name: '修改索引 idx_name', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog.getByLabel('索引定义')).toHaveValue("(`name`(20) DESC) COMMENT 'keep'")
  await dialog.getByLabel('索引名称').fill('idx_display')
  await dialog.getByLabel('索引类型').selectOption('unique')
  await dialog.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page.getByRole('alertdialog')).toContainText('放弃未保存的结构变更')
  await page.getByRole('alertdialog').getByRole('button', { name: '取消', exact: true }).click()
  await expect(dialog.getByLabel('索引名称')).toHaveValue('idx_display')
  await page.getByRole('button', { name: '关闭 users', exact: true }).click({ force: true })
  await expect(page.getByRole('alertdialog')).toContainText('未保存的结构变更')
  await page.getByRole('alertdialog').getByRole('button', { name: '取消', exact: true }).click()
  await page.evaluate(() => { (window as any).__DBM_SCHEMA_FAILURE__ = true })
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await expect(dialog.locator('.schema-preview')).toContainText('ADD UNIQUE INDEX `idx_display`')
  await page.screenshot({ path: 'test-results/schema-editor.png' })
  await dialog.getByRole('button', { name: '执行修改' }).click()
  await expect(dialog.getByRole('alert')).toContainText('Duplicate entry')
  await expect(dialog.getByLabel('索引名称')).toHaveValue('idx_display')
  await expect(dialog.getByRole('button', { name: '预览 SQL' })).toBeEnabled()
  await page.evaluate(() => { (window as any).__DBM_SCHEMA_FAILURE__ = false })
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await dialog.getByRole('button', { name: '执行修改' }).click()
  await expect(page.getByRole('cell', { name: 'idx_display', exact: true })).toBeVisible()
})

test('schema editing blocks dirty data and views, and handles stale metadata', async ({ page }) => {
  await openTable(page)
  await page.getByRole('button', { name: '新增行', exact: false }).click()
  await page.getByRole('button', { name: '字段', exact: true }).click()
  await expect(page.getByRole('button', { name: '修改字段 name', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: '新增字段', exact: false })).toBeDisabled()
  await expect(page.getByRole('button', { name: '删除字段 name', exact: true })).toBeDisabled()
  await expect(page.getByText('请先保存或撤销待提交的数据，再修改表结构。')).toBeVisible()
  await page.getByRole('button', { name: '索引', exact: true }).click()
  await expect(page.getByRole('button', { name: '新增索引', exact: false })).toBeDisabled()
  await expect(page.getByRole('button', { name: '修改索引 idx_name', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: '删除索引 idx_name', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: '数据', exact: true }).click()
  await page.getByRole('button', { name: '撤销', exact: true }).click()
  await page.getByRole('button', { name: '撤销变更', exact: true }).click()
  await page.getByRole('button', { name: '字段', exact: true }).click()
  await page.getByRole('button', { name: '修改字段 name', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('字段名称').fill('renamed')
  await page.evaluate(() => { (window as any).__DBM_SCHEMA_CONFLICT__ = true })
  await dialog.getByRole('button', { name: '预览 SQL' }).click()
  await expect(dialog.getByRole('alert')).toContainText('表结构已改变')
  await expect(dialog.getByRole('button', { name: '执行修改' })).toHaveCount(0)
  await dialog.getByRole('button', { name: '关闭并刷新结构' }).click()
  await page.getByRole('button', { name: '放弃变更', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await page.evaluate(() => { (window as any).__DBM_VIEW__ = true })
  await page.getByTitle('刷新数据与结构').click()
  await expect(page.getByText('视图结构请通过 SQL 修改。')).toBeVisible()
  await expect(page.getByRole('button', { name: '修改字段 name', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: '新增字段', exact: false })).toBeDisabled()
  await expect(page.getByRole('button', { name: '删除字段 name', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: '索引', exact: true }).click()
  await expect(page.getByRole('button', { name: '新增索引', exact: false })).toBeDisabled()
  await expect(page.getByRole('button', { name: '修改索引 idx_name', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: '删除索引 idx_name', exact: true })).toBeDisabled()
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
