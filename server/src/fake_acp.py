# A minimal ACP agent for botd's Rust tests: same `session/update` shapes
# observed on `copilot --acp` and `opencode acp`. One mode per test, passed as
# the first argument. Reads JSON-RPC 2.0 on stdin, writes on stdout, exits when
# the scripted turn is done. Test fixture only — never shipped or run in prod.
import json
import sys

MODE = sys.argv[1] if len(sys.argv) > 1 else "ok"


def req(rid, method, params):
    return {"jsonrpc": "2.0", "id": rid, "method": method, "params": params}


def notify(method, params):
    return {"jsonrpc": "2.0", "method": method, "params": params}


def respond(rid, result):
    return {"jsonrpc": "2.0", "id": rid, "result": result}


def chunk(text, mid="m1"):
    return notify("session/update", {"sessionId": "s1", "update": {
        "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text},
        "messageId": mid}})


def out(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()


def prompt_response(rid):
    out(respond(rid, {"stopReason": "end_turn", "usage": {"totalTokens": 1}}))


for line in sys.stdin:
    try:
        o = json.loads(line)
    except ValueError:
        continue
    method = o.get("method", "")
    rid = o.get("id")
    params = o.get("params", {})

    if method == "initialize":
        if MODE == "bad_handshake":
            out({"jsonrpc": "2.0", "id": rid,
                 "error": {"code": -32000, "message": "auth required: run the login"}})
            break
        out(respond(rid, {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}))

    elif method == "session/new":
        out(notify("session/update", {"sessionId": "s1", "update": {
            "sessionUpdate": "session_info_update", "title": "test"}}))
        out(respond(rid, {"sessionId": "s1"}))

    elif method == "session/load":
        if MODE == "load_fails":
            out({"jsonrpc": "2.0", "id": rid,
                 "error": {"code": -32002, "message": "Resource not found: Session gone not found"}})
            break
        # Copilot replays the old conversation during load, and answers
        # without a sessionId. Both are covered here.
        out(chunk("REPLAYED", mid="m_old"))
        out(respond(rid, {}))

    elif method == "session/prompt":
        if MODE == "permission":
            # Ask before finishing; only proceed once botd approves.
            out(req("p1", "session/request_permission", {
                "sessionId": "s1",
                "options": [{"optionId": "allow_once", "kind": "allow_once", "name": "Allow"}]}))
            approved = False
            for ans_line in sys.stdin:
                try:
                    ans = json.loads(ans_line)
                except ValueError:
                    continue
                approved = (ans.get("result", {}).get("outcome", {}).get("outcome") == "selected"
                            and ans.get("result", {}).get("outcome", {}).get("optionId") == "allow_once")
                break
            out(chunk("approved" if approved else "denied"))
            prompt_response(rid)
        elif MODE == "tool":
            out(notify("session/update", {"sessionId": "s1", "update": {
                "sessionUpdate": "tool_call", "toolCallId": "call_1", "title": "List files",
                "kind": "execute", "status": "pending",
                "rawInput": {"command": "ls /tmp", "description": "List files"}}}))
            out(chunk("listed"))
            prompt_response(rid)
        else:
            # Echo the session id the prompt was sent with: proves resume uses it.
            out(chunk("prompt_for:" + str(params.get("sessionId")), mid="m_reply"))
            prompt_response(rid)
        if MODE == "exit_130":
            # OpenCode's real behaviour: exits 130 once the client kills it.
            sys.exit(130)
        if MODE == "lingers":
            # Real ACP agents are servers: they keep waiting after the turn.
            for _ in sys.stdin:
                pass
        break
