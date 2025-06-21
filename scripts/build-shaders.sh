#!/bin/bash

SHADER_DIR="resources/shaders"
OUTPUT_DIR="target/shaders"

mkdir -p "$OUTPUT_DIR"

echo "Compiling Vulkan shaders..."

for shader_file in "$SHADER_DIR"/*.vert "$SHADER_DIR"/*.frag "$SHADER_DIR"/*.comp "$SHADER_DIR"/*.glsl; do
    if [[ -f "$shader_file" ]]; then
        filename=$(basename "$shader_file")
        output_file="$OUTPUT_DIR/${filename}.spv"
        
        echo "Compiling $filename..."
        glslc "$shader_file" -o "$output_file"
        
        if [[ $? -eq 0 ]]; then
            echo "✓ Compiled $filename"
        else
            echo "✗ Failed to compile $filename"
            exit 1
        fi
    fi
done

echo "All shaders compiled successfully!"