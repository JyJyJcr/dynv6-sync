#!/bin/sh
set -xeu

# validate
echo "[VALIDATE]"
if [ -z "$1" ];then
    exit 1
else
    target="$1"
fi
if [ -e "cicd/target/$target/triple" ];then
    triple="$(cat "cicd/target/$target/triple")"
else
    exit 1
fi
if [ "$2" = "release" ];then
    is_release=yes
else
    is_release=no
fi

# update
echo "[UPDATE]"
if [ -e "cicd/target/$target/update.sh" ];then
    echo exec update.sh
    "cicd/target/$target/update.sh"
    ret=$?
    if [ $ret -ne 0 ];then
        echo "update.sh failed"
        exit $ret
    fi
else
    echo no update.sh
fi
rustup update
cargo install-update -a

# build
echo "[BUILD]"
if [ "$is_release" = "yes" ];then
    cargo build --target "$triple" --release
else
    cargo build --target "$triple"
fi

# echo "[TEST]"
# cargo test --target "$triple" --release

echo "[EXPORT]"
mkdir export
if [ "$is_release" = "yes" ];then
    if [ -e "cicd/target/$target/release.sh" ];then
        "cicd/target/$target/release.sh" "$triple"
        exit $?
    else
        echo no release.sh
        exit 0
    fi
else
    cp target/$triple/debug/dynv6-sync export/dynv6-sync
fi
