#!/usr/bin/env python3
"""Pretty-print emthin's `state` message: the figures, their pages and rects."""
import json
import sys

text = sys.stdin.read()
dec = json.JSONDecoder()
objs, idx = [], 0
while idx < len(text):
    while idx < len(text) and text[idx] != "{":
        idx += 1
    if idx >= len(text):
        break
    try:
        o, end = dec.raw_decode(text, idx)
    except ValueError:
        break
    objs.append(o)
    idx = end

states = [o for o in objs if o.get("method") == "state"]
if not states:
    print("no state message")
    sys.exit(1)
st = states[-1]["params"]
print(f"page {st['page']} of {st['page_count']}")
for f in st["figures"]:
    r = f["rect"]
    print(
        f"  {f['key']:<4} caption={f['caption']!r:<12} id={f['id']!r:<8} "
        f"page={f['page']} rect={r['w']}x{r['h']} @ ({r['x']},{r['y']}) "
        f"window={f['window_id']} title={f['title']!r}"
    )