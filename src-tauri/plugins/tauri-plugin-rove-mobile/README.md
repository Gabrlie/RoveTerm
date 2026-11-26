# RoveTerm 移动桥接

应用外壳注册 `init()` 后，通过 `MobileBridgeExt::rove_mobile()` 访问原生能力。密钥操作和外链打开应在工作线程调用，系统界面由平台主线程处理。默认权限集为空，WebView 通过应用层接口使用这些能力。

主密钥为 32 字节，按服务名隔离。Android 将 AES-GCM 密文写入应用私有存储，包装密钥保存在 Android Keystore。iOS 将密钥写入设备专用的 Keychain 条目。静默读取受生物识别或设备凭据保护的密钥时返回 `MobileError::Denied`。服务尚未写入密钥时返回 `None`。

`get_insets()` 读取最近一次原生更新，初值为零。`rove-mobile://insets` 发布安全区域和键盘底部高度，Android 使用 dp，iOS 使用 point。

`get_network()`、`on_network_change()` 与 `on_lifecycle()` 在工作线程分发经校验去重的原生网络快照（Android 通过 `ConnectivityManager` 默认网络回调，iOS 通过 `NWPathMonitor` 与 `getifaddrs`）与前后台生命周期事件，供用户态 Tailscale 感知切网与回前台恢复。

Android 侧还提供 `install_apk(path)`：仅允许应用缓存目录内的常规 `.apk` 文件，先检查未知来源安装权限（未授权时跳转系统设置并在返回后自动续装），授权后通过 `FileProvider` 拉起系统包安装器。
