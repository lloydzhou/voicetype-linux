BINARY_NAME := voicetype
PREFIX := $(HOME)/.local
BIN_DIR := $(PREFIX)/bin
DATA_DIR := $(PREFIX)/share/voicetype
MODEL_DIR := $(DATA_DIR)/models
MODEL_URL := https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
MODEL_FILE := $(MODEL_DIR)/ggml-base.bin

SENSEVOICE_DIR := $(MODEL_DIR)/sensevoice
# Same files as the GitHub release tarball
# (k2-fsa/sherpa-onnx asr-models release), served via hf-mirror, which is
# reachable where GitHub release downloads stall. Use
# `make model-sensevoice HF=https://huggingface.co` if you prefer HF directly.
HF ?= https://hf-mirror.com
SENSEVOICE_REPO := csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17

.PHONY: build run install model model-sensevoice clean

build:
	cargo build --release

run: build
	cargo run --release

install: build
	install -Dm755 target/release/$(BINARY_NAME) $(BIN_DIR)/$(BINARY_NAME)
	@echo "Installed to $(BIN_DIR)/$(BINARY_NAME)"

model:
	mkdir -p $(MODEL_DIR)
	curl -L --fail -o $(MODEL_FILE) $(MODEL_URL)
	@echo "Model saved to $(MODEL_FILE)"

# SenseVoiceSmall INT8 (zh/en/ja/ko/yue): model.int8.onnx ~228 MB + tokens.txt.
model-sensevoice:
	mkdir -p $(SENSEVOICE_DIR)
	curl -L --fail --retry 3 -o $(SENSEVOICE_DIR)/model.int8.onnx \
		$(HF)/$(SENSEVOICE_REPO)/resolve/main/model.int8.onnx
	curl -L --fail --retry 3 -o $(SENSEVOICE_DIR)/tokens.txt \
		$(HF)/$(SENSEVOICE_REPO)/resolve/main/tokens.txt
	@echo "SenseVoice model saved to $(SENSEVOICE_DIR)/"

clean:
	cargo clean
