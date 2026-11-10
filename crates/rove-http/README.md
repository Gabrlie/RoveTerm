# rove-http

构建 HTTP 客户端的唯一入口：reqwest + rustls（ring），使用系统证书；Android 上系统校验器需先经 JNI 初始化，改用 `webpki-roots` 内置的 Mozilla 根证书（ALPN 固定 http/1.1），只信任用户自行安装的 CA 的服务器在 Android 上会被拒绝。

## 契约

- `client(timeout)` 返回带 RoveTerm user agent 的客户端；`timeout` 为整次请求的上限，`None` 不限（流式响应自行限时）。
- `builder(timeout)` 返回同样设置的构造器，供需要额外设置（如重定向策略）的调用方使用。
- 重新导出 `reqwest`，调用方不必单独声明版本。
