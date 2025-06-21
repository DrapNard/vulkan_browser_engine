FROM rust:1.70-slim as builder

WORKDIR /app
COPY . .

RUN apt-get update && apt-get install -y \
    build-essential \
    cmake \
    pkg-config \
    libfontconfig1-dev \
    vulkan-tools \
    libvulkan-dev \
    vulkan-validationlayers-dev \
    && rm -rf /var/lib/apt/lists/*

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libvulkan1 \
    libfontconfig1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/vulkan-renderer /usr/local/bin/
COPY --from=builder /app/resources /usr/local/share/vulkan-renderer/

EXPOSE 8080
CMD ["vulkan-renderer"]
