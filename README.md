# voicetype-linux

Linux 按住说话、松开上屏的语音输入工具。项目仓库：[lloydzhou/voicetype-linux](https://github.com/lloydzhou/voicetype-linux)。项目源码采用 [MIT 许可证](LICENSE)，第三方组件与模型另见 [许可说明](THIRD_PARTY_NOTICES.md)。

## 使用方式与当前状态

- 同时按住**右 Ctrl + 右 Shift** 开始录音，HUD 显示麦克风电平与阶段性识别结果；**松开任意一个键即提交**。不区分松键顺序，没有按键取消操作，左侧同名按键不触发。
- 默认使用 SenseVoiceSmall INT8，在本地对已累计音频反复执行离线推理以更新文字，**不是真正的流式识别**；松键后生成最终文本。SenseVoice 路径达到约 30 秒音频上限也会自动结束并提交。
- LLM 润色默认关闭。启用后，识别文字会发送到配置的 OpenAI 兼容服务；请先确认服务隐私政策和费用。模型可能误识别或改写内容，不保证准确率、无幻觉或固定延迟。
- X11 已在 Arch Linux、nwm、Sakura 环境进行测试，但不代表“快捷键 → 麦克风 → 识别 → 目标窗口粘贴”的实时完整路径在所有环境已验证。请先在无敏感内容的文本框中试用。
- **Wayland 为实验性支持**：注入依赖 `wl-clipboard`、`wtype` 和合成器的虚拟键盘协议支持；HUD 未实现 layer-shell，不能保证位置、置顶及不抢焦点。不要据此假定 GNOME、KDE 或其他合成器可用。

### 快捷键配置

`hotkeys` 数组配置按住说话的组合，任一组合完全按下即开始录音，松开其中任意一个键提交。可用键名：`LeftCtrl` `RightCtrl` `LeftShift` `RightShift` `LeftAlt` `RightAlt` `LeftSuper` `RightSuper`（或 `LeftWin`/`RightWin`）`Space` `CapsLock`，也可直接写 evdev 键码（如 `"100"`）。默认 `["RightCtrl+RightShift"]`；可配置多个组合，例如 `hotkeys = ["RightCtrl+RightShift", "RightAlt"]`。键名写错会在启动时报错退出。改配置后需重启程序生效。

快捷键通过 evdev 读取，不会屏蔽窗口管理器或输入法对相同组合的处理：`Ctrl+Shift` 可能触发输入法切换，单键（如 `RightAlt`）会占用该键的正常输入功能，请自行权衡。文本通过剪贴板粘贴到注入时的焦点窗口，X11 对识别出的终端使用 `Ctrl+Shift+V`，其他应用使用 `Ctrl+V`。不会自动切换输入法；应用自身的快捷键、剪贴板管理器及焦点变化都可能影响结果。日志中的命令成功不等于目标应用已收到文字。

## 安装

### Debian / Ubuntu

首个版本 [v0.1.0](https://github.com/lloydzhou/voicetype-linux/releases/tag/v0.1.0) 已发布，提供 **AMD64（x86_64）** `.deb`，在 Ubuntu 22.04、Ubuntu 24.04 和 Debian 12 的干净容器中完成安装验证。已接入共享 APT 源：

```bash
echo "deb [trusted=yes] https://lloydzhou.github.io/bash-agent/debian stable main" \
    | sudo tee /etc/apt/sources.list.d/lloydzhou.list
sudo apt update && sudo apt install voicetype
```

也可以从 [GitHub Release](https://github.com/lloydzhou/voicetype-linux/releases/latest) 下载 `voicetype_*_amd64.deb` 后 `sudo apt install ./voicetype_*_amd64.deb`。该源当前未签名（`trusted=yes`）。模型与输入权限仍需按下文单独配置。

### 从源码构建（Arch Linux）

需要 Rust/Cargo、C/C++ 构建工具、GTK3、ALSA 和托盘依赖。X11 示例：

```bash
sudo pacman -S --needed rust cmake clang base-devel alsa-lib gtk3 \
    libappindicator-gtk3 xdotool xclip xorg-xwininfo curl
cargo build --release
```

实验性 Wayland 注入另需 `wl-clipboard` 和 `wtype`。

### Debian 发布流程

[Debian release](https://github.com/lloydzhou/voicetype-linux/actions/workflows/release.yml) 在 main 推送时验证构建，也可手动运行。推送 `v0.1.0` 这样的版本标签会在验证成功后创建 GitHub Release，上传 AMD64 `.deb` 和 SHA256SUMS；已接入的共享 APT 源需在 [bash-agent](https://github.com/lloydzhou/bash-agent) 侧手动运行一次 `apt-republish` 刷新，新版本不会自动出现。

构建使用 Ubuntu 22.04；GTK、ALSA、X11 等由 APT 依赖安装，匹配构建版本的 sherpa-onnx / ONNX Runtime 放在包内 `/usr/lib/voicetype/`，不覆盖系统库。CI 在干净的 Ubuntu 22.04 和 Debian 12 容器中安装检查，不继承构建缓存或 `LD_LIBRARY_PATH`。容器检查不等于真实桌面的麦克风、热键和粘贴验证。Arch 请使用本机构建，不安装 Debian 包。

发布后下载对应包并运行 `sudo apt install ./voicetype_*_amd64.deb`；模型及输入权限仍需按下文单独配置。共享 APT 源尚未接入。

**不要只复制可执行文件就假定安装完成。** 通过 `.deb` 安装会自动处理系统库与包内推理库；但源码构建（如 Arch）需要运行时能找到 sherpa-onnx / ONNX Runtime 的 `.so`，且模型、输入权限仍要单独配置。不建议通过以 root 身份运行来解决运行库或输入权限问题。

### 输入设备权限

运行用户需要读取 evdev 键盘设备。若发行版以 `input` 组管理这些权限，可在理解风险后手动授权：

```bash
sudo usermod -aG input "$USER"
# 注销并重新登录后生效
```

**安全警告：`input` 组通常可访问全部输入设备，不仅是这个快捷键，可能读取其他应用中的键盘输入（包括密码）。** 请按发行版的设备权限策略选择授权方式，仅授权可信用户。程序应以普通用户运行；安装与运行不应自动提权，也不要用 `sudo voicetype` 绕过权限设置。

## 模型与配置

模型需要联网单独下载，下载前请核对上游许可证。默认引擎需要 `model.int8.onnx` 与 `tokens.txt`。Debian 包提供下载命令；请以运行听写的普通用户执行：

```bash
voicetype-download-model
voicetype-download-model --hf-base https://hf-mirror.com
voicetype-download-model --proxy http://127.0.0.1:7890
voicetype-download-model --output-dir /path/to/models/sensevoice
```

默认 HF 源为 `HF_ENDPOINT` 或 `https://huggingface.co`；也遵循 curl 的代理环境变量。默认目录为 `${XDG_DATA_HOME:-$HOME/.local/share}/voicetype/models/sensevoice`。自定义目录后需同步配置 `sensevoice_dir`。两个文件均下载并通过非空／非 HTML 检查后才替换旧文件，失败保留原文件；替换是逐文件重命名，不保证两文件整体原子性。该检查不能证明模型真实性，请使用可信源。

源码用户可运行 `bash scripts/download-model.sh`，参数相同，或使用 Make：

```bash
make model-sensevoice HF=https://huggingface.co
# 可选：Whisper 的 ggml-base.bin，仅 whisper 路径需要
make model
```

下载目标默认是 `$HOME/.local/share/voicetype/models/`。`make` 的安装前缀与程序使用的 XDG 数据目录可能不同；自定义 `PREFIX` 或 `XDG_DATA_HOME` 时，请核对实际文件位置并填写配置。

配置默认位于 `$HOME/.config/voicetype/config.toml`（遵循 `XDG_CONFIG_HOME`），首次运行尝试生成。可参考 [配置示例](config.example.toml)。**配置字符串中的 `~` 和环境变量不会展开**，请使用实际绝对路径，例如将下列 `YOURUSER` 替换为用户名：

```toml
language = "zh-Hans"        # en | zh-Hans | zh-Hant | ja | ko
engine = "sensevoice"       # sensevoice | whisper
sensevoice_dir = "/home/YOURUSER/.local/share/voicetype/models/sensevoice"
model_path = "/home/YOURUSER/.local/share/voicetype/models/ggml-base.bin"
hotkeys = ["RightCtrl+RightShift"]  # 自定义快捷键，见下文
ime_switch = false          # 当前粘贴路径不切换输入法

[llm]
enabled = false
base_url = "https://api.openai.com/v1"
api_key = ""
model = "gpt-4o-mini"
```

API Key 会保存在本地配置中，请保护文件权限，不要提交配置或含敏感识别内容的日志。托盘提供语言及 LLM 设置；LLM 设置中的连接测试会发出真实网络请求。简繁转换为字符映射近似处理。

## 运行与排查

共享库、模型和输入权限就绪后，在 Linux 桌面会话中以普通用户运行：

```bash
./target/release/voicetype
RUST_LOG=debug ./target/release/voicetype
# 单独检查 WAV 转写，不验证快捷键、实时录音、HUD 或粘贴
./target/release/voicetype --transcribe-sv test.wav
./target/release/voicetype --transcribe test.wav
```

`--transcribe-sv` 使用 SenseVoice，`--transcribe` 始终使用 Whisper。遇到问题时，请提供发行版、X11/Wayland、窗口管理器、目标应用、引擎以及去除隐私信息后的错误日志，并区分文件转写成功与实时完整路径成功。
