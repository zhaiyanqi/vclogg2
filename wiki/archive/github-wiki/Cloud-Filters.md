# 预定义与云端过滤器

本地日志阅读和搜索不需要云端连接。云端功能用于过滤器目录、分享和修订协作；需要单独部署或接入兼容服务，不能把一个普通网址当成兼容 API。

## 本地过滤器

“工具 → 预定义过滤器”提供新增、编辑、排序、导入与导出。过滤器包含稳定 UUID、名称、匹配内容和相关属性。同内容不同 UUID 是独立分支，不按名称合并；导入同 UUID 则遵循更新规则。

当前 JSON 导出为 v5；读取兼容早期 v1–v4、数组格式及旧版 Qt CONF/INI。导入文件未包含的本地过滤器不会因此删除。搜索栏弹层勾选过滤器用于组合查询，详见[搜索与过滤](https://github.com/zhaiyanqi/vclogg2/wiki/Search-and-Filters)。

## 配置连接

在“设置 → 网络”填写服务器连接信息并保存/测试，然后进入过滤器的云端页面。现有实现使用 `/api/v1` 数据契约，首次注册不依赖客户端内置签名私钥；会话用于按需 HTTP 请求，不建立常驻长连接。

公开连接配置写入 SQLite。Cookie/CSRF 保存在本地私有会话文件，文件内容没有额外加密，不使用系统钥匙串；不要把这些文件附到 Issue、Wiki 或过滤器分享中。

## 下载与协作

| 状态 | 处理方式 |
| --- | --- |
| 本地尚无此 UUID | 下载并保留云端 UUID |
| 只有远端修改 | 更新本地到新修订 |
| 两边改了不同字段 | 三方合并，保留两边改动 |
| 两边改了同一字段 | 显式选择本地/远端或逐字段处理 |
| 暂不解决冲突 | “稍后处理”，保留本地和共同基线 |
| 想创建独立分支 | “另存为新过滤器”，生成新 UUID |

合并到本地不代表已提交到远端。提交本地修改使用共同修订基线；服务器返回并发修订冲突时读取最新版并合并，不自动再次推送。历史“恢复到本地”只形成待提交修改；删除远端分享保留本地过滤器。

## 兼容性与离线

服务未声明 `filter-uuid-branches-v1` 时仍可浏览、缓存和下载，但首次分享和远程提交禁用。连接失败时可查看此前浏览过的公开分页缓存，在本机搜索、排序和翻页；这不是服务器全部目录的离线副本。

离线只允许安全下载/合入比本地基线更新的缓存修订。点赞、分享、提交、远端删除及修订读取/恢复等联网写入或读取流程不可用。详细发行前检查见[过滤器分支验收](https://github.com/zhaiyanqi/vclogg2/wiki/Filter-Branch-Acceptance)。


源码依据：[cloud_filters.rs](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/crates/vclogg-app/src/cloud_filters.rs) · [predefined_filters.rs](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/crates/vclogg-app/src/predefined_filters.rs) · [predefined_filters_dialog.rs](https://github.com/zhaiyanqi/vclogg2/blob/65fa1d4fa7f44984825fa84758515a03d2157aeb/crates/vclogg-app/src/predefined_filters_dialog.rs)。
