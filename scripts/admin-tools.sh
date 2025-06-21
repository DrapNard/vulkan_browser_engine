#!/bin/bash

case "$1" in
    "monitor")
        echo "Starting performance monitoring..."
        cargo build --release
        perf record -g ./target/release/vulkan-renderer
        perf report
        ;;
    "profile-memory")
        echo "Profiling memory usage..."
        valgrind --tool=massif ./target/release/vulkan-renderer
        ms_print massif.out.* > memory_profile.txt
        ;;
    "trace-gpu")
        echo "Tracing GPU operations..."
        renderdoc-cli --capture-file gpu_trace.rdc ./target/release/vulkan-renderer
        ;;
    "analyze-deps")
        echo "Analyzing dependencies..."
        cargo tree --duplicates
        cargo audit
        ;;
    "clean-all")
        echo "Cleaning build artifacts..."
        cargo clean
        rm -rf target/
        rm -f *.rdc *.out.*
        ;;
    *)
        echo "Usage: $0 {monitor|profile-memory|trace-gpu|analyze-deps|clean-all}"
        exit 1
        ;;
esac