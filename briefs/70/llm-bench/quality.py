"""One filing, one call: a title and a one-sentence summary, as JSON, from the
llama-server at 127.0.0.1:$PORT. Prints a markdown table of times and answers.
Usage: python3 -I quality.py <samples dir> <model label> <port>"""
import json, os, sys, time, urllib.request
samples, label, port = sys.argv[1], sys.argv[2], sys.argv[3]
SCHEMA = {"type": "object", "properties": {
    "title": {"type": "string", "maxLength": 80},
    "summary": {"type": "string", "maxLength": 200}}, "required": ["title", "summary"]}
ASK = ("\n\nAbove is the start of a public company filing. Give a short, specific title for it "
       "(a noun phrase, at most 8 words, naming what it announces or contains), and one sentence "
       "of at most 20 words saying what it announces. Answer as JSON with keys title and summary.")
def chat(text):
    body = {"messages": [{"role": "user", "content": "FILING TEXT:\n" + text + ASK}],
            "temperature": 0.1, "max_tokens": 120, "cache_prompt": True,
            # a model with a reasoning mode (Qwen3.5) answers directly, as every other one does
            "chat_template_kwargs": {"enable_thinking": False},
            "response_format": {"type": "json_object", "schema": SCHEMA}}
    req = urllib.request.Request(f"http://127.0.0.1:{port}/v1/chat/completions", json.dumps(body).encode(),
                                 {"Content-Type": "application/json"})
    t = time.time(); r = json.load(urllib.request.urlopen(req, timeout=3600)); dt = time.time() - t
    u = r.get("usage", {})
    try:
        a = json.loads(r["choices"][0]["message"]["content"]); title, summary = a.get("title", ""), a.get("summary", "")
    except Exception:
        title, summary = "(not JSON)", r["choices"][0]["message"]["content"][:120]
    return dt, u.get("prompt_tokens"), u.get("completion_tokens"), title, summary
for f in sorted(os.listdir(samples)):
    if not f.endswith(".txt"):
        continue
    dt, pt, ct, title, summary = chat(open(os.path.join(samples, f)).read())
    print(f"| {label} | {f} | {dt:.1f} s | {pt}/{ct} | {title} | {summary} |", flush=True)
