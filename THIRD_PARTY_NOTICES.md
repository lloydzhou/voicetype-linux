# 第三方组件与模型许可

voicetype-linux 自有源码采用 [MIT 许可证](LICENSE)。这不将第三方代码、原生库或模型重新许可为 MIT。以下为主要上游入口，不是完整依赖许可证清单，也不能代替分发时需要附带的许可证与版权声明。

## 软件依赖

直接 Rust 依赖见 [Cargo.toml](Cargo.toml)，具体解析版本及传递依赖见 [Cargo.lock](Cargo.lock)。主要组件包括：

| 组件 | 用途 | 上游许可入口 |
| --- | --- | --- |
| sherpa-rs / sherpa-rs-sys | sherpa-onnx Rust 绑定及原生库集成 | [thewh1teagle/sherpa-rs](https://github.com/thewh1teagle/sherpa-rs) |
| sherpa-onnx | SenseVoice 离线推理 | [k2-fsa/sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)（Apache-2.0） |
| ONNX Runtime | ONNX 模型运行时 | [microsoft/onnxruntime](https://github.com/microsoft/onnxruntime)（MIT；随附第三方声明另计） |
| whisper-rs / whisper.cpp | Whisper Rust 绑定与原生推理 | [whisper-rs](https://codeberg.org/tazz4843/whisper-rs)、[ggml-org/whisper.cpp](https://github.com/ggml-org/whisper.cpp) |
| GTK3、GLib、Pango、Cairo 及 gtk-rs 绑定 | 桌面界面与绘制 | [GTK](https://www.gtk.org/)、[gtk-rs](https://gtk-rs.org/)、[Cairo](https://www.cairographics.org/)；绑定和底层库许可证分别适用 |
| tray-icon、evdev、cpal | 托盘、输入设备、音频 | [tray-icon](https://github.com/tauri-apps/tray-icon)、[evdev](https://github.com/emberian/evdev)、[cpal](https://github.com/RustAudio/cpal) |

其他依赖包括 hound、reqwest、serde、serde_json、toml、log、env_logger、dirs 及其传递依赖。外部命令（如 xdotool、xclip、xwininfo、wl-clipboard、wtype）与系统音频、图形及托盘库也遵循各自许可证。请检查实际使用版本所附的 LICENSE、COPYING、NOTICE 和包元数据，不要将 Rust 绑定的许可证等同于其链接原生库的许可证。

构建可能下载 sherpa-onnx / ONNX Runtime 预编译库。发布可执行文件、共享库或 `.deb` 前，分发者应核对实际包含的组件，附带所需许可证、版权与第三方声明，并履行适用的源码提供等义务。本文件不是已完成二进制发行合规审计的声明。

## 单独下载的模型

模型权重与词表不随本项目源码或安装包捆绑；下载脚本只是获取入口，不授予额外使用或再分发权利。

- **SenseVoiceSmall / sherpa-onnx 转换模型**：查看 [FunAudioLLM/SenseVoice](https://github.com/FunAudioLLM/SenseVoice) 及实际下载的 [SenseVoice 模型仓库](https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17) 的模型卡、许可证、词表与转换说明。
- **Whisper / GGML 模型**：查看 [OpenAI Whisper](https://github.com/openai/whisper) 及 [whisper.cpp 模型仓库](https://huggingface.co/ggerganov/whisper.cpp) 的权重许可和转换说明；换用其他模型时重新核对相应许可。

用户和分发者须自行核对下载版本的上游条款，尤其是商业使用、再分发、模型修改和署名要求。不能因为推理引擎开源，就假定所有模型具有相同许可证或无使用限制。
