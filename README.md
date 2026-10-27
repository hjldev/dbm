# DBM

DBM 是基于 **Tauri 2 + Vue 3 + Rust** 的轻量 MySQL 桌面工作台，首发平台为 macOS Apple Silicon（macOS 13 及以上）。中文界面包含连接树、多标签 SQL 编辑器、结构查看和可编辑数据表，主题默认跟随系统。

## 开发与打包

需要 Node.js 22.12+（推荐 24 LTS）、npm、Rust stable 和 Xcode Command Line Tools。

```sh
npm install
npm run tauri -- dev
```

项目使用 Node.js 自带的 npm，依赖版本记录在 `package-lock.json` 中。已有锁文件时可使用 `npm ci` 重现安装。

数据库能力只在 Tauri 桌面应用内可用；`npm run dev` 仅用于浏览器界面开发。应用不附带 MySQL 服务，需要连接已有 MySQL 8.0 或 8.4。

```sh
npm run build
npm run tauri -- build
```

`npm run build` 仅生成前端静态资源；`npm run tauri -- build` 使用 Tauri 标准流程，自动构建前端、原生应用并生成 `.app` 和 `.dmg`。`npm run release:mac` 是同等打包入口；如需单独指定产物，可使用 `npm run build:app` 或 `npm run build:dmg`，两者均直接调用 Tauri。

如果 DMG 步骤提示未获授权向 Finder 发送 Apple 事件，可授权当前终端控制 Finder，或使用 `CI=true npm run tauri -- build` 通过 Tauri 原生 CI 模式跳过安装窗口美化。

本机产物位于 `src-tauri/target/release/bundle/macos/DBM.app` 和 `src-tauri/target/release/bundle/dmg/`。当前包采用本地 ad-hoc 签名用于安装验证，尚未配置 Developer ID 签名、公证或自动更新。

## 使用

1. 新建连接，填写主机、端口、用户名、密码及可选默认库；可先测试连接。默认勾选“记住密码”，密码以明文保存在本机配置文件中，断开重连或重启应用后自动使用。启用 TLS 会校验证书与主机名，可通过文件对话框导入 PEM CA 证书。
2. 展开连接和数据库，按需加载表与视图；可搜索、刷新并查看字段、索引、外键和建表 SQL。
3. 新建 SQL 标签，执行光标所在语句、选中脚本或整个脚本；支持 `.sql` 文件打开和保存、语法高亮及库表字段补全。每个 SQL 标签使用独立会话，保留 `USE`、会话变量和显式事务。
4. 打开表数据，使用分页、排序和条件筛选。新增、编辑、删除先进入待提交状态，可撤销；保存时以一笔事务提交整批变更。更新和删除会检查原始行，发现并发修改则整批回滚。
5. SQL 结果可导出已保留的预览；表数据可按当前筛选导出全表。全表 CSV 导出通过 Rust 流式写入，显示进度并支持取消。
6. 在“字段”或“索引”页点击“新增字段 / 新增索引”，或使用行内的“修改 / 删除”。新增与修改可填写名称和完整定义；索引可选择主键、唯一、普通、全文或空间类型。定义保留字符集、默认表达式、生成字段、前缀长度、排序及索引选项。先预览 SQL，再执行；删除还需确认删除对象，删除字段会移除此字段的全部数据。完成后自动刷新结构和数据。存在待提交数据时需先保存或撤销，视图结构使用 SQL 修改。结构修改立即生效，不能通过数据页“撤销”回滚；服务端约束和权限错误会保留编辑内容，检测到表结构变化时需刷新后重试。

图形编辑需要 InnoDB 表及主键或非空唯一键；视图、无可用唯一键的表和其他引擎保持只读。生成字段不可修改。SQL 可直接执行用户有权限的其他操作。

配置位于 `~/Library/Application Support/dev.dbm.desktop/settings.json`，界面设置中会显示完整路径。`connections` 保存连接参数，`passwords` 按连接 ID 保存明文密码，文件权限为当前用户读写（0600）；修改文件前先退出 DBM。编辑连接时不填写密码会保留原密码，明确输入空密码会将其设为空；取消“记住密码”或删除连接会移除配置中的密码。已保存密码只在 Rust 后端使用，不通过 IPC 返回界面。

旧版记住的 Keychain 密码在首次成功连接或编辑保存时迁入配置文件；如旧凭据无法读取，可重新输入一次。新连接不访问钥匙串，旧钥匙串条目不会自动删除，用户可自行清理。

脚本按语句顺序执行，遇错停止，显示错误所在语句及源码行号。默认自动提交，显式事务由 SQL 管理，脚本报错后保留事务状态；关闭标签时会处理尚未提交的事务。第一版不支持 `DELIMITER` 与存储程序定义，也不包含 SSH、图形建表、备份恢复或数据同步。

取消操作对应正在执行的数据库线程，使用独立控制连接发送 `KILL QUERY`，并在查询清理完成后允许后续执行；取消不会撤销已经提交的语句。断线或会话失效不会自动重放 SQL。

CSV 为 UTF-8，包含列名并按 CSV 规则转义逗号、引号和换行。NULL 表示为 `\N`，二进制表示为 `base64:<编码>`；大整数及 DECIMAL 保留字符串精度。查询导出只使用保留的预览，不重新执行 SQL。文本值恰好等于 `\N` 或 `base64:...` 时，导入端应结合字段类型识别。

## 内存与架构

- WebView 承载 Vue，Rust 承担数据库访问、凭据存储和文件读写；前端不直接连接 MySQL。
- 元数据按需加载，元数据及表格连接池上限为 2，SQL 会话按标签创建并在关闭时释放。
- 表数据默认每页 200 行，通过服务端排序与筛选读取，行使用虚拟滚动；不把整张表载入前端。
- 查询默认预览 1,000 行，可在设置中调整；超出预览的行继续在 Rust 端流式消费，界面明确标记截断。
- 仅最近 10 个语句结果保留行预览，其余保留执行摘要；每个查询标签的执行记录保留最近 1,000 条语句，执行事件按批次刷新界面；编辑器按需加载，大结果避免深层响应式代理。
- 全表 CSV 导出直接写文件，不向前端传输所有行；关闭标签释放结果缓存。

内部 Tauri IPC 使用类型化请求与响应，`CellValue` 区分 NULL、文本、整数、DECIMAL、浮点数、日期和二进制；所有整数和精确数值采用字符串传递。数据编辑、筛选通过参数绑定执行，数据库、表和字段名先校验再转义。文件选择由原生对话框处理，应用采用所需的最小 Tauri capabilities。

## 验证

```sh
npm run typecheck
npm test
npm run test:ui
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

字段与索引的真实数据库测试可通过 `DBM_SCHEMA_TEST_HOST`、`DBM_SCHEMA_TEST_PORT`、`DBM_SCHEMA_TEST_USER`、`DBM_SCHEMA_TEST_PASSWORD` 配置本地 MySQL，再运行 `cargo test --manifest-path src-tauri/Cargo.toml --test schema_integration -- --ignored`。账号需有建库权限；测试创建随机名称的独立数据库，结束后清理，不依赖其他 fixture 或已有业务表。

界面自动化使用本机 Google Chrome；完整结果与原生操作记录见 [VALIDATION.md](VALIDATION.md)。旧版 Keychain 兼容测试可单独运行 `cargo test --manifest-path src-tauri/Cargo.toml --test keychain_integration -- --ignored`，只创建并删除自己的随机测试条目。


```sh
默认 `cargo test` 跳过需要数据库的测试；可用 `DBM_TEST_PORTS` 指定上述 fixture 的端口，`DBM_TEST_PASSWORD` 覆盖测试密码。请勿将 fixture 或集成测试指向用户数据库。

测试覆盖元数据、复合主键、非空唯一键、只读表、外键、服务端分页与排序筛选、中文/JSON/二进制/NULL/大整数/DECIMAL/日期、生成字段、批量回滚及并发冲突；以及脚本解析、SQL mode、变量、显式事务、遇错停止、多结果集及后续结果错误、预览截断后协议清理、重复与延迟取消、会话中断、认证失败、TLS 验证和权限不足。图标源文件位于 `assets/icon.svg`，可用 `npm run tauri -- icon assets/icon.svg` 重新生成。
