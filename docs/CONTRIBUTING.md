# Contributing Guide

## Development Setup

1. Install Rust toolchain (1.70+)
2. Install Vulkan SDK
3. Clone repository and run `./install.sh`

## Code Style

- Use `rustfmt` for formatting
- Follow Rust naming conventions
- Add documentation for public APIs
- Write unit tests for new features

## Testing

```bash
cargo test --all-features
cargo bench
cargo clippy --all-targets
```

## Pull Request Process

1. Fork the repository
2. Create feature branch
3. Write tests for new functionality
4. Ensure all tests pass
5. Submit pull request with detailed description

## Performance Guidelines

- Profile critical paths with `perf`
- Use `criterion` for benchmarks
- Minimize allocations in hot paths
- Prefer zero-copy operations