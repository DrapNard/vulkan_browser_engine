#!/bin/bash
set -e

echo "Installing Vulkan Renderer Engine dependencies..."

if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    sudo apt update
    sudo apt install -y build-essential cmake pkg-config libfontconfig1-dev
    sudo apt install -y vulkan-tools libvulkan-dev vulkan-validationlayers-dev spirv-tools
    sudo apt install -y libxcb1-dev libxrandr-dev libxss-dev libxcursor-dev libxcomposite-dev libasound2-dev libpulse-dev
elif [[ "$OSTYPE" == "darwin"* ]]; then
    brew install vulkan-headers vulkan-loader vulkan-tools
    brew install spirv-tools
elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "win32" ]]; then
    echo "Please install Vulkan SDK from https://vulkan.lunarg.com/"
    echo "And Visual Studio with C++ tools"
fi

echo "Installing Rust if not present..."
if ! command -v rustc &> /dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source ~/.cargo/env
fi

rustup default stable
rustup component add clippy rustfmt

echo "Building project..."
cargo build --release

echo "Installation complete!"