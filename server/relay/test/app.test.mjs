import test from "node:test";
import assert from "node:assert/strict";
import { createRelayApp } from "../src/app.mjs";
import { signRequest } from "../src/security.mjs";

const NOW = 1_787_600_000_000;
const SECRET = "a-secure-shared-secret-that-is-long-enough";
const config = {
  desktopSharedSecret: SECRET,
  telegramWebhookSecret: "telegram-webhook-secret",
  allowedTelegramUserIds: new Set(["123456789"]),
  maxBodyBytes: 16_384,
  maxClockSkewSeconds: 300,
  leaseSeconds: 90,
  maxRequestsPerMinute: 60,
  jobRetentionHours: 24,
};

function repository() {
  const documents = new Map();
  return {
    documents,
    async createOnce(collection, id, record) {
      const key = `${collection}/${id}`;
      if (documents.has(key)) return null;
      const value = { ...record, updateTime: `time-${documents.size}` };
      documents.set(key, value);
      return value;
    },
    async claimOldest(deviceId) {
      const entry = [...documents.entries()].find(([key, value]) => key.startsWith("relay_jobs/") && value.status === "queued");
      if (!entry) return null;
      entry[1].status = "leased";
      entry[1].leasedBy = deviceId;
      entry[1].leaseExpiresAtMs = NOW + 90_000;
      return entry[1];
    },
    async patch(collection, id, record) {
      const key = `${collection}/${id}`;
      const current = documents.get(key);
      if (!current) { const error = new Error("missing"); error.status = 404; throw error; }
      const value = { ...current, ...record };
      documents.set(key, value);
      return value;
    },
    async get(collection, id) {
      return documents.get(`${collection}/${id}`) ?? null;
    },
  };
}

function signedRequest(pathname, payload, nonce = "nonce_1234567890abcdef") {
  const body = JSON.stringify(payload);
  const timestamp = String(NOW);
  return {
    method: "POST", pathname, body, remoteAddress: "127.0.0.1",
    headers: {
      "x-investa-timestamp": timestamp,
      "x-investa-nonce": nonce,
      "x-investa-signature": signRequest(SECRET, { timestamp, nonce, method: "POST", pathname, body }),
    },
  };
}

test("telegram webhook rejects unknown users and creates an idempotent job", async () => {
  const store = repository();
  const messages = [];
  const app = createRelayApp({ config, repository: store, telegram: { sendMessage: async (...args) => messages.push(args) }, now: () => NOW });
  const update = { update_id: 42, message: { from: { id: 123456789 }, chat: { id: 123456789 }, text: "한화 분석해줘" } };
  const request = { method: "POST", pathname: "/telegram/webhook", body: JSON.stringify(update), remoteAddress: "telegram", headers: { "x-telegram-bot-api-secret-token": config.telegramWebhookSecret } };
  assert.equal((await app(request)).status, 200);
  assert.equal((await app(request)).status, 200);
  assert.equal(messages.length, 1);
  assert.equal(store.documents.get("relay_jobs/telegram-42").instruction, "한화 분석해줘");
  assert.equal(store.documents.get("relay_jobs/telegram-42").expiresAtMs, NOW + 24 * 3_600_000);

  update.message.from.id = 999;
  assert.equal((await app({ ...request, body: JSON.stringify(update) })).status, 403);
});

test("telegram webhook rejects secrets before creating a Firestore job", async () => {
  const store = repository();
  const app = createRelayApp({ config, repository: store, telegram: { sendMessage: async () => {} }, now: () => NOW });
  const update = { update_id: 43, message: { from: { id: 123456789 }, chat: { id: 123456789 }, text: "api_key=abcdefghijklmnopqrstuvwxyz123456" } };
  const response = await app({ method: "POST", pathname: "/telegram/webhook", body: JSON.stringify(update), remoteAddress: "telegram", headers: { "x-telegram-bot-api-secret-token": config.telegramWebhookSecret } });
  assert.equal(response.status, 400);
  assert.equal(JSON.parse(response.body).error, "sensitive_instruction");
  assert.equal(store.documents.has("relay_jobs/telegram-43"), false);
});

test("desktop pull requires a valid non-replayed signature", async () => {
  const store = repository();
  await store.createOnce("relay_jobs", "telegram-1", {
    jobId: "telegram-1", sourceRequestId: "update:1", sourceUserId: "123456789", sourceChatId: "123456789",
    instruction: "상태 알려줘", status: "queued", createdAtMs: NOW, updatedAtMs: NOW,
  });
  const app = createRelayApp({ config, repository: store, telegram: { sendMessage: async () => {} }, now: () => NOW });
  const request = signedRequest("/v1/jobs/pull", { deviceId: "desktop-1" });
  const first = await app(request);
  assert.equal(first.status, 200);
  assert.equal(JSON.parse(first.body).job.jobId, "telegram-1");
  assert.equal((await app(request)).status, 409);
  assert.equal((await app({ ...request, headers: { ...request.headers, "x-investa-signature": "0".repeat(64) } })).status, 401);
});

test("oversized bodies are rejected before authentication or JSON parsing", async () => {
  const app = createRelayApp({ config, repository: repository(), telegram: { sendMessage: async () => {} }, now: () => NOW });
  const response = await app({ method: "POST", pathname: "/telegram/webhook", body: "{}", tooLarge: true, remoteAddress: "x", headers: {} });
  assert.equal(response.status, 413);
});

test("result endpoint reports local approval without enabling live orders", async () => {
  const store = repository();
  await store.createOnce("relay_jobs", "telegram-9", {
    jobId: "telegram-9", sourceRequestId: "update:9", sourceUserId: "123456789", sourceChatId: "123456789",
    instruction: "한화 매수해", status: "leased", leasedBy: "desktop-1", leaseExpiresAtMs: NOW + 90_000, createdAtMs: NOW, updatedAtMs: NOW,
  });
  const messages = [];
  const app = createRelayApp({ config, repository: store, telegram: { sendMessage: async (...args) => messages.push(args) }, now: () => NOW });
  const response = await app(signedRequest("/v1/jobs/telegram-9/result", {
    deviceId: "desktop-1", localJobId: "remote:1", status: "awaiting_local_approval", resultText: "로컬 승인 필요",
  }, "nonce_result_1234567890"));
  assert.equal(response.status, 200);
  assert.equal(messages[0][1], "PC의 로컬 승인을 기다립니다.");
});

test("result endpoint refuses to forward or store secrets", async () => {
  const store = repository();
  await store.createOnce("relay_jobs", "telegram-10", {
    jobId: "telegram-10", sourceRequestId: "update:10", sourceUserId: "123456789", sourceChatId: "123456789",
    instruction: "상태 알려줘", status: "leased", leasedBy: "desktop-1", leaseExpiresAtMs: NOW + 90_000, createdAtMs: NOW, updatedAtMs: NOW,
  });
  const messages = [];
  const app = createRelayApp({ config, repository: store, telegram: { sendMessage: async (...args) => messages.push(args) }, now: () => NOW });
  const response = await app(signedRequest("/v1/jobs/telegram-10/result", {
    deviceId: "desktop-1", localJobId: "remote:2", status: "completed", resultText: "authorization: Bearer abcdefghijklmnopqrstuvwxyz",
  }, "nonce_result_secret_1234"));
  assert.equal(response.status, 400);
  assert.equal(messages.length, 0);
  assert.equal(store.documents.get("relay_jobs/telegram-10").resultText, undefined);
});
