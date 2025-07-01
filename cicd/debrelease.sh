#!/bin/sh
if [ -z "$1" ];then
    exit 70
fi
if [ -z "$2" ];then
    exit 71
fi
cargo deb --target "$2" --variant "$1"
for deb in $(ls "target/$2/debian"|grep -E '\.deb$') ;do
    cp "target/$2/debian/$deb" "export/$deb"
    cat /etc/os-release|grep VERSION_CODENAME|sed -e "s/^.*=//g" > "export/$deb.codename"
done
