#!/usr/bin/env python3
"""First-party Steam Guard TOTP generator — stdlib only, no third-party dependency.

Steam's TOTP is NOT RFC-6238 numeric: it emits a 5-char code over the alphabet
"23456789BCDFGHJKMNPQRTVWXY". Reads the base64 authenticator seed from argv[1] or
the STEAM_SHARED_SECRET env var. Relies on an accurate clock (GitHub runners are
NTP-synced, so no offset handling needed)."""
import base64, hmac, hashlib, struct, sys, time


def steam_guard_code(shared_secret, for_time=None):
    key = base64.b64decode(shared_secret)
    counter = int((for_time if for_time is not None else time.time()) // 30)
    digest = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
    offset = digest[19] & 0x0F
    code_int = struct.unpack(">I", digest[offset:offset + 4])[0] & 0x7FFFFFFF
    alphabet = "23456789BCDFGHJKMNPQRTVWXY"
    out = ""
    for _ in range(5):
        out += alphabet[code_int % len(alphabet)]
        code_int //= len(alphabet)
    return out


if __name__ == "__main__":
    secret = sys.argv[1] if len(sys.argv) > 1 else __import__("os").environ["STEAM_SHARED_SECRET"]
    print(steam_guard_code(secret))
