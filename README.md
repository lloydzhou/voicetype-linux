# voicetype-linux

Linux 按住说话、松开上屏的语音输入工具。项目仓库：[lloydzhou/voicetype-linux](https://github.com/lloydzhou/voicetype-linux)。项目源码采用 [MIT 许可证](LICENSE)，第三方组件与模型另见 [许可说明](THIRD_PARTY_NOTICES.md)。

## 使用方式与当前状态

- 同时按住**右 Ctrl + 右 Shift** 开始录音，HUD 显示麦克风电平与阶段性识别结果；**松开任意一个键即提交**。不区分松键顺序，没有按键取消操作，左侧同名按键不触发。
- 默认使用 SenseVoiceSmall INT8，在本地对已累计音频反复执行离线推理以更新文字，**不是真正的流式识别**；松键后生成最终文本。SenseVoice 路径达到约 30 秒音频上限也会自动结束并提交。
- LLM 润色默认关闭。启用后，识别文字会发送到配置的 OpenAI 兼容服务；请先确认服务隐私政策和费用。模型可能误识别或改写内容，不保证准确率、无幻觉或固定延迟。
- X11 已在 Arch Linux、nwm、Sakura 环境进行测试，但不代表“快捷键 → 麦克风 → 识别 → 目标窗口粘贴”的实时完整路径在所有环境已验证。请先在无敏感内容的文本框中试用。
- **Wayland 为实验性支持**：注入依赖 `wl-clipboard`、`wtype` 和合成器的虚拟键盘协议支持；HUD 未实现 layer-shell，不能保证位置、置顶及不抢焦点。不要据此假定 GNOME、KDE 或其他合成器可用。

快捷键通过 evdev 读取，不会屏蔽窗口管理器的同名绑定；请避免冲突。文本通过剪贴板粘贴到注入时的焦点窗口，X11 对识别出的终端使用 `Ctrl+Shift+V`，其他应用使用 `Ctrl+V`。不会自动切换输入法；应用自身的快捷键、剪贴板管理器及焦点变化都可能影响结果。日志中的命令成功不等于目标应用已收到文字。

## 安装

### Debian / Ubuntu

计划通过 GitHub Release 工作流提供 `.deb`，初期面向 **AMD64（x86_64）**。发布产物及共享 APT 仓库集成尚待验证，目前不提供未经验证的 APT 地址或一键安装命令。模型单独下载，不随源码或安装包捆绑。

### 从源码构建（Arch Linux）

需要 Rust/Cargo、C/C++ 构建工具、GTK3、ALSA 和托盘依赖。X11 示例：

```bash
sudo pacman -S --needed rust cmake clang base-devel alsa-lib gtk3 \
    libappindicator-gtk3 xdotool xclip xorg-xwininfo curl
cargo build --release
```

实验性 Wayland 注入另需 `wl-clipboard` 和 `wtype`。Debian/Ubuntu 的构建依赖与运行库打包仍待安装验证。

**不要只复制可执行文件就假定安装完成。** 当前 sherpa-onnx / ONNX Runtime 路径需要共享库，运行时必须能找到兼容的 `.so`。共享库安装与打包仍在完善；构建成功不等于目标机器可直接运行。请以验证后的发布说明为准，不建议通过以 root 身份运行来解决运行库或输入权限问题。

### 输入设备权限

运行用户需要读取 evdev 键盘设备。若发行版以 `input` 组管理这些权限，可在理解风险后手动授权：

```bash
sudo usermod -aG input "$USER"
# 注销并重新登录后生效
```

**安全警告：`input` 组通常可访问全部输入设备，不仅是这个快捷键，可能读取其他应用中的键盘输入（包括密码）。** 请按发行版的设备权限策略选择授权方式，仅授权可信用户。程序应以普通用户运行；安装与运行不应自动提权，也不要用 `sudo voicetype` 绕过权限设置。

## 模型与配置

模型需要联网单独下载，下载前请核对上游许可证。默认引擎需要 `model.int8.onnx` 与 `tokens.txt`：

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
hotkey = "RightCtrl+RightShift (hold to talk)" # 仅说明，不改变快捷键
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
