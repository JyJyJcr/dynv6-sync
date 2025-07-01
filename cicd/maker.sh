#!/bin/sh
set -xeu

# validate
echo "[VALIDATE]"
if [ -z "$1" ];then
    exit 1
else
    target="$1"
fi
if [ -z "$2" ];then
    exit 2
else
    triple="$2"
fi
if [ "$3" = "release" ];then
    is_release=yes
else
    is_release=no
fi

# build
echo "[BUILD]"

if [ "$is_release" = "yes" ];then
    cargo build --target "$triple" --release
else
    cargo build --target "$triple"
fi

# echo "[TEST]"
# cargo test --target "$triple" --release

if [ "$is_release" = "yes" ];then
    echo "[RELEASE]"
    if [ -e "cicd/target/$target/gen.sh" ];then
        "cicd/target/$target/gen.sh" "$triple"
        exit $?
    else
        echo no gen.sh
        exit 0
    fi
fi
