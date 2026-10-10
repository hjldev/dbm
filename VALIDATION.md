# DBM 0.1.0 验收记录

验收日期：2026-10-10。平台：本机 macOS Apple Silicon。数据库：隔离 Docker fixture 中的 MySQL 8.0、8.4。

## 自动化验证

| 检查 | 结果 |
| --- | --- |
| Vue / TypeScript 类型检查及 Vite 生产构建 | 通过 |
| npm 安装与锁文件 | `npm ci` 全新安装通过，直接依赖版本沿用迁移前版本 |
| 前端数据与会话行为测试 | 14 项通过 |
| Chrome 界面测试 | 7 项通过，包括默认记住密码、重连、编辑留空保留密码、配置路径显示、虚拟滚动、批量编辑、关闭保护和 SQL IPC |
| Rust 单元及配置持久化测试 | 16 项单元测试、6 项配置集成测试通过 |
| MySQL 集成测试 | 6 组在 8.0、8.4 均通过；断线关闭新增回归也在两版通过 |
| 旧版 macOS Keychain 兼容测试 | 2 项通过，包含迁入配置文件并移除测试钥匙串条目后重新加载；随机测试条目已删除 |
| Rust 全目标 Clippy | `-D warnings` 通过 |
| Release 应用 | arm64，使用本地 ad-hoc 签名 |
| DMG | 生成成功，磁盘映像校验通过，包含 DBM.app 和 Applications 快捷方式 |

MySQL 测试覆盖认证失败、TLS 信任与主机名校验、权限不足、连接中断、多种 SQL mode、引号与注释、会话变量、事务、脚本遇错停止、多结果集及延迟错误、预览截断后继续查询、重复/延迟取消与取消失败竞争。数据测试覆盖键选择、复合键、NULL、中文、JSON、二进制、大整数、DECIMAL、生成字段、分页排序筛选、并发冲突及约束失败整批回滚。

密码存储调整已回归：默认保存到 `settings.json`，清除内存缓存及重新加载应用状态后仍能读取；编辑保留原密码、显式空密码、取消记住及删除连接、写入失败时磁盘与内存不变、旧配置兼容与钥匙串迁移均通过。文件权限核对为 0600，启动 IPC 只返回连接参数、界面设置和文件路径。此项变更未重跑 MySQL 全量集成和此前的原生数据库操作流程。

## 原生应用操作

在实际 Release `.app` 中完成：连接测试与保存、认证失败后重试、数据库/表/视图加载、200 行分页和翻页、双击单元格编辑并提交、未保存变更阻止翻页、多语句执行、会话变量、查询取消及取消后继续执行、显式事务、SQL 文件保存、⌘W 关闭标签、⌘Q 未保存 SQL 与事务回滚提示，以及取消退出后继续使用会话。

实际 CSV 文件核对：查询预览导出为表头加 1,000 行；整表导出为表头加 10,000 行，首尾记录正确。慢速视图导出取消后没有生成半成品目标文件。

修复了原生 WebKit 搜索框布局，以及 macOS 默认 Quit 绕过 Tauri `ExitRequested` 的问题。退出保护通过补充 `applicationShouldTerminate:` 将正常退出转入同一窗口关闭检查；升级 Tao/Tauri 时需重新回归菜单退出、Dock 退出和快捷键。背景见 [Tauri 原生退出回调问题](https://github.com/tauri-apps/tauri/issues/12978)。

## 资源验证边界

查询保留最多 10 个结果集的行预览（包含同一语句返回多个结果集的情况）。界面测试连续打开、执行并关闭 12 个查询标签，确认每轮编辑器、结果面板及标签节点清空，后端会话打开/关闭调用数均为 12。虚拟表格的 DOM 行数显著少于单页数据行数。

这些验证覆盖资源边界与释放路径，尚未作为长时间运行的 RSS/峰值内存基准；第一版没有内存阈值验收结论。

## 复现

```sh
npm run typecheck
npm test
npm run test:ui # 使用本机 Google Chrome；测试自带模拟 IPC
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
sh scripts/test-mysql.sh
cargo test --manifest-path src-tauri/Cargo.toml --test keychain_integration -- --ignored
npm run release:mac
```

验收完成后，隔离的 MySQL 测试容器已停止并移除。应用内保留的“DBM 本地验收”连接仅指向该测试环境，未保存密码；可删除或运行测试脚本重建环境。

安装包用于本地验证，尚未进行 Developer ID 签名、公证或其他平台验收。
