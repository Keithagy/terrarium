#!/usr/bin/env python3
"""A stand-in for `claude -p` that answers the discovery agents' prompts without
spending anything: it streams a few `Read` tool uses for the files named in the
prompt, then a schema-shaped answer. Point TERRARIUM_CLAUDE at it to exercise the
live discovery UI and the verify loop.

    TERRARIUM_CLAUDE=scripts/stand-in-claude.py terrarium discover
    TERRARIUM_CLAUDE=scripts/stand-in-claude.py terrarium app launch fixtures/polyglot

STANDIN_DELAY (seconds between reads, default 0.25) paces the stream so the UI has
something to show.
"""
import json
import os
import re
import sys
import time

DELAY = float(os.environ.get("STANDIN_DELAY", "0.25"))


def arg(flag, default=None):
    args = sys.argv[1:]
    return args[args.index(flag) + 1] if flag in args else default


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def read(path):
    emit({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "input": {"file_path": os.path.join(os.getcwd(), path)}}]}})
    time.sleep(DELAY)


def listed_files(prompt):
    """`1. path (n lines)` lines in a container prompt."""
    return re.findall(r"^\s*\d+\. (\S+) \(\d+ lines\)", prompt, re.M)


def packages(prompt):
    """`- name (dir `x`): n files, guessed kind in tech` lines in the survey prompt."""
    return re.findall(r"^- (\S+) \(dir `([^`]*)`\): (\d+) files, guessed ([a-z ]+?) in", prompt, re.M)


def words(name):
    return " ".join(w.capitalize() for w in re.split(r"[-_]", name) if w)


KIND = {"web app": "web", "desktop app": "desktop", "service": "service", "worker": "worker", "command line": "cli", "library": "library", "tooling": "tooling"}


def survey(prompt):
    for f in ["README.md", "package.json", "Cargo.toml", "pyproject.toml", "go.mod"]:
        if os.path.exists(f):
            read(f)
    pk = packages(prompt)
    containers = []
    for name, d, files, kind in pk:
        k = KIND.get(kind.strip(), "library")
        hidden = int(files) == 0 or k == "tooling"
        containers.append({"package": name, "name": f"{words(name)} {'tooling' if hidden else {'web': 'front end', 'desktop': 'desktop shell', 'service': 'service', 'worker': 'worker', 'cli': 'command line', 'library': 'library'}[k]}", "kind": k, "technology": "as scanned", "description": f"The `{name}` package, read by a stand-in agent.", "hidden": hidden})
    entries = re.findall(r"^- (\S+#\S+) \(\d+ boundaries", prompt, re.M)
    usable = [c["package"] for c in containers if not c["hidden"]]
    return {
        "system": {"name": f"{words(os.path.basename(os.getcwd()))} Town", "purpose": "A stand-in survey of this repository.", "summary": "A stand-in agent surveyed the packages, named each container and picked the journeys; nothing here was read for meaning."},
        "people": [{"name": "Developer", "description": "Opens the repository and follows the journeys.", "uses": [{"container": c, "how": "works in"} for c in usable[:2]]}],
        "externals": [{"name": "PostgreSQL", "kind": "database", "description": "Where the rows live.", "used_by": [{"container": usable[0], "how": "keeps its rows in", "technology": "SQL"}]}] if usable else [],
        "containers": containers,
        "journeys": [{"entry": e, "name": f"Follow {e.split('#')[-1]} to the end"} for e in entries[:3]],
    }


def container(prompt):
    files = listed_files(prompt)
    for f in files:
        read(f)
    first, rest = files[0], files[1:]
    components = [{"name": "Entry point", "description": f"Where `{os.path.basename(first)}` starts things.", "technology": "", "responsibilities": ["start the container"], "files": [first]}]
    if rest:
        components.append({"name": "Helpers", "description": "Everything the entry point leans on.", "technology": "", "responsibilities": ["do the work"], "files": rest})
    return {
        "description": f"A container of {len(files)} files, grouped by a stand-in agent into an entry point and its helpers.",
        "technology": "as scanned",
        "responsibilities": ["answer requests", "keep its data"],
        "components": components,
        "relationships": [{"from": "Entry point", "to": "Helpers", "label": "leans on", "technology": "function call"}, {"from": "Helpers", "to": "Entry point", "label": "hands results back to", "technology": ""}],
    }


def journey(prompt):
    m = re.search(r"steps: ([\d, ]+)\.", prompt)
    wanted = [int(x) for x in m.group(1).split(",")] if m else []
    for path in re.findall(r"^\d+\. +(\S+?)(?::\d+)?(?: <-|\s|$)", prompt, re.M)[:4]:
        read(path.split("#")[0])
    return {"name": "Follow one request end to end", "summary": "A stand-in narrator followed the calls the scanner found. Each step below is one crossing or one place the data comes to rest.", "captions": [{"step": i, "caption": f"Step {i}: the call goes on, says the stand-in."} for i in wanted]}


def editor(prompt):
    names = re.findall(r"^- (.+?) \(", prompt, re.M)
    return {"summary": "A stand-in editor wrote this summary from the field notes: the containers listed above work together, and the journeys show how.", "start_here": [{"element": n, "why": "A stand-in says to start here."} for n in names[:2]], "callouts": [{"title": "This atlas was written by a stand-in", "detail": "Run a real discovery for words that mean something.", "element": names[0] if names else ""}]}


def main():
    prompt = arg("-p", "")
    emit({"type": "system", "subtype": "init", "model": arg("--model", "stand-in")})
    if prompt.startswith("You are surveying"):
        answer = survey(prompt)
    elif prompt.startswith("You are describing one container"):
        answer = container(prompt)
    elif prompt.startswith("You are narrating"):
        answer = journey(prompt)
    elif prompt.startswith("You are editing"):
        answer = editor(prompt)
    else:
        emit({"type": "result", "subtype": "error", "is_error": True, "result": "stand-in does not know this prompt", "total_cost_usd": 0})
        return
    time.sleep(DELAY * 2)
    emit({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0.07, "structured_output": answer})


if __name__ == "__main__":
    main()
