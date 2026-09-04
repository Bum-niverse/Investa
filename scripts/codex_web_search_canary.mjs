import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const executable = process.argv[2] || process.env.INVESTA_CODEX_PATH;
if (!executable) {
  throw new Error("Codex 실행 파일 경로를 첫 번째 인자 또는 INVESTA_CODEX_PATH로 전달하세요.");
}

const child = spawn(executable, ["app-server", "--stdio"], {
  stdio: ["pipe", "pipe", "pipe"],
  windowsHide: true,
});
const lines = createInterface({ input: child.stdout });
let nextId = 1;
const pending = new Map();
let sawWebSearch = false;
let visibleText = "";

const send = (method, params) => new Promise((resolve, reject) => {
  const id = nextId++;
  pending.set(id, { resolve, reject });
  child.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
});

const timeout = setTimeout(() => {
  child.kill();
  throw new Error("Codex 웹 검색 canary가 120초 안에 끝나지 않았습니다.");
}, 120_000);

lines.on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  if (typeof message.id === "number" && pending.has(message.id)) {
    const waiter = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) waiter.reject(new Error(message.error.message || "Codex JSON-RPC 오류"));
    else waiter.resolve(message.result);
    return;
  }
  if (typeof message.method === "string" && message.method.startsWith("item/") && JSON.stringify(message.params).match(/web[_A-Z]?search/i)) {
    sawWebSearch = true;
  }
  if (message.method === "item/agentMessage/delta" && typeof message.params?.delta === "string") {
    visibleText += message.params.delta;
  }
  if (message.method === "turn/completed") {
    clearTimeout(timeout);
    const passed = sawWebSearch && /https:\/\//.test(visibleText);
    process.stdout.write(`${JSON.stringify({ passed, sawWebSearch, returnedHttpsUrl: /https:\/\//.test(visibleText) }, null, 2)}\n`);
    child.kill();
    process.exitCode = passed ? 0 : 1;
  }
});

child.stderr.on("data", () => {});
child.on("error", (error) => {
  clearTimeout(timeout);
  throw error;
});

await send("initialize", {
  clientInfo: { name: "investa-canary", title: "Investa Codex Web Canary", version: "0.1.0" },
  capabilities: { experimentalApi: false },
});
child.stdin.write(`${JSON.stringify({ method: "initialized" })}\n`);
const thread = await send("thread/start", {
  cwd: process.cwd(),
  approvalPolicy: "never",
  sandbox: "read-only",
  config: {
    web_search: "live",
  },
  ephemeral: true,
  developerInstructions: "공식 공개 문서만 웹 검색하세요. 파일·shell·로그인·외부 작업은 금지됩니다.",
});
const threadId = thread?.thread?.id;
if (!threadId) throw new Error("thread/start 응답에서 thread ID를 찾지 못했습니다.");
await send("turn/start", {
  threadId,
  input: [{ type: "text", text: "웹 검색을 사용해 OpenAI Codex 공식 문서 URL 하나만 확인하고 짧게 답하세요." }],
  approvalPolicy: "never",
  sandboxPolicy: { type: "readOnly", networkAccess: false },
});
