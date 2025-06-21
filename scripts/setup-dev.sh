#!/bin/bash
set -e

echo "Setting up development environment..."

if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    sudo apt update
    sudo apt install -y build-essential cmake pkg-config
    sudo apt install -y vulkan-tools libvulkan-dev vulkan-validationlayers-dev
    sudo apt install -y spirv-tools glslang-tools
    sudo apt install -y libxcb1-dev libxrandr-dev libxss-dev
    sudo apt install -y valgrind perf linux-tools-generic
elif [[ "$OSTYPE" == "darwin"* ]]; then
    brew install vulkan-headers vulkan-loader vulkan-tools
    brew install spirv-tools glslang
    brew install llvm
fi

rustup component add rustfmt clippy llvm-tools-preview
cargo install cargo-audit cargo-criterion cargo-expand

echo "Installing git hooks..."
cat > .git/hooks/pre-commit << 'EOF'
#!/bin/bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
EOF
chmod +x .git/hooks/pre-commit

echo "Development environment setup complete!"