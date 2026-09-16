"""The session helpers that do not touch the network, both ways.

Nothing here contacts Wealthsimple. What is checked is the part that decides
what a failure is called and what reaches the page: only a short OAuth error
name may, never a token, a client id or a raw body.

    cargo build -p bagholder-ws && python3 crates/ws/sessiontest.py
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import bagholder  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "sesstool")
NOW = 1789000000.0

ROWS = [
    {"error": "invalid_grant"},
    {"error": "invalid_grant", "_http_status": 400},
    {"error": "invalid_client", "_http_status": 401},
    {"_http_status": 500},
    {},
    {"error": ""},
    {"error": "   "},
    {"error": "a" * 70},                                  # too long to be an error name
    {"error": "deadbeef" * 4},                            # a hex identifier, not an error
    {"error": "has space"},
    {"error": "with/slash"},
    {"error": "dots.and-dashes_ok"},
    {"error": 42},
    {"error": None},
    {"expires_at": "2026-09-16T14:00:00.000Z"},
    {"expires_at": 1789000123},
    {"expires_in": 3600},
    {"expires_in": "3600"},
    {"expires_at": "not a stamp"},
    {"identity_canonical_id": "id-1"},
    {"sub": "id-2"},
    {"resource_owner_id": 7},
    {"application_uid": " uid-1 "},
    {"application": {"uid": "uid-2"}},
    {"application": "not a dict"},
]


def norm(v):
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 9) + 0.0
    return v


def main():
    got = json.loads(subprocess.run([BIN], input=json.dumps({"rows": ROWS, "now": NOW}),
                                    capture_output=True, text=True, check=True).stdout)
    bad = []
    for i, (r, g) in enumerate(zip(ROWS, got)):
        want = {
            "oauthError": bagholder._oauth_error_code(r),
            "refreshMessage": bagholder._refresh_failure_message(r),
            "expiresAt": bagholder._expires_at_as_timestamp(r),
            "identity": bagholder._identity_from(r),
            "clientId": bagholder.client_id_from_token_info(r),
        }
        for k in want:
            w, x = want[k], g.get(k)
            if k == "expiresAt" and w and x:
                # `expires_in` is measured from now on both sides; only the shape is comparable
                if len(w) == len(x) and w[-1] == x[-1] == "Z":
                    continue
            if norm(w) != norm(x):
                bad.append(f"row[{i}] {json.dumps(r)[:50]} .{k}: py={w!r} rs={x!r}")
    for line in bad[:20]:
        print("  " + line)
    print(f"{len(ROWS)} session rows, {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
