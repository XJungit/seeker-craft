#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""补齐 Loom 的 Minecraft 客户端/服务端 jar 到 Gradle loom 缓存，
避免 loom 的 Java 直连下载器在本机网络上卡死。带 SHA1 校验。"""
import hashlib
import os
import sys
import urllib.request

LOOM_CACHE = os.path.expanduser(
    r"~\.gradle\caches\fabric-loom\1.21.11"
)

FILES = [
    # (url, 期望sha1, 目标文件名)
    ("https://piston-data.mojang.com/v1/objects/ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd/client.jar",
     "ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd", "minecraft-client.jar"),
    ("https://piston-data.mojang.com/v1/objects/64bb6d763bed0a9f1d632ec347938594144943ed/server.jar",
     "64bb6d763bed0a9f1d632ec347938594144943ed", "minecraft-server.jar"),
]


def sha1_of(path):
    h = hashlib.sha1()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    os.makedirs(LOOM_CACHE, exist_ok=True)
    for url, expected, name in FILES:
        dest = os.path.join(LOOM_CACHE, name)
        part = dest + ".part"
        # 清理可能存在的半成品
        if os.path.exists(part):
            os.remove(part)
        if os.path.exists(dest):
            if sha1_of(dest) == expected:
                print(f"[skip] {name} 已存在且校验通过")
                continue
            else:
                print(f"[warn] {name} 校验失败，重新下载")
                os.remove(dest)
        print(f"[get ] {name} <- {url}")
        try:
            urllib.request.urlretrieve(url, part)
            os.rename(part, dest)
        except Exception as e:
            print(f"[FAIL] {name}: {e}")
            sys.exit(1)
        got = sha1_of(dest)
        if got != expected:
            print(f"[FAIL] {name} SHA 不匹配 got={got} exp={expected}")
            sys.exit(1)
        print(f"[ok  ] {name} ({os.path.getsize(dest)} bytes, sha ok)")
    print("ALL_DONE")


if __name__ == "__main__":
    main()
