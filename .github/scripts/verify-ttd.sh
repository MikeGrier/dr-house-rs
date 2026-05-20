#!/usr/bin/env bash
# Verify that TTD binaries are present and accessible

set -e

TTD_DIR="./extension/resources/ttd"

echo "TTD Binary Verification"
echo "======================"

if [ ! -d "$TTD_DIR" ]; then
    echo "✗ TTD directory not found: $TTD_DIR"
    exit 1
fi

echo "✓ TTD directory found"

platforms=("x64" "x86" "arm64")
required_dlls=("TTDReplay.dll" "TTDReplayCPU.dll")

all_good=true

for platform in "${platforms[@]}"; do
    platform_dir="$TTD_DIR/$platform"
    
    if [ ! -d "$platform_dir" ]; then
        echo "✗ Platform directory missing: $platform_dir"
        all_good=false
        continue
    fi
    
    echo "  $platform:"
    for dll in "${required_dlls[@]}"; do
        dll_path="$platform_dir/$dll"
        if [ -f "$dll_path" ]; then
            size=$(stat -f%z "$dll_path" 2>/dev/null || stat -c%s "$dll_path" 2>/dev/null || echo "?")
            echo "    ✓ $dll ($size bytes)"
        else
            echo "    ✗ $dll missing"
            all_good=false
        fi
    done
done

if [ "$all_good" = false ]; then
    echo ""
    echo "Some TTD binaries are missing."
    echo "Run: npm run download-ttd"
    exit 1
fi

echo ""
echo "✓ All TTD binaries verified"
