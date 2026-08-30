import crypto from "node:crypto";
import { containsSecretMarker, verifyDesktopRequest, verifyTelegramSecret, sha256Hex } from "./security.mjs";

const jsonResponse = (status, payload) => ({ status, headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" }, body: JSON.stringify(payload) });
const validDeviceId = (value) => typeof value === "string" && /^[A-Za-z0-9_.:-]{1,128}$/.test(value);

class RateLimiter {
  constructor(limit) { this.limit = limit; this.buckets = new Map(); }
  allow(key, nowMs) {
    const minute = Math.floor(nowMs / 60_000);
    if (this.buckets.size > 2_048) {
      for (const [bucketKey, bucket] of this.buckets) {
        if (bucket.minute < minute) this.buckets.delete(bucketKey);
      }
      if (this.buckets.size > 2_048 && !this.buckets.has(key)) return false;
    }
    const current = this.buckets.get(key);
    if (!current || current.minute !== minute) { this.buckets.set(key, { minute, count: 1 }); return true; }
    current.count += 1;
    return current.count <= this.limit;
  }
}

export function createRelayApp({ config, repository, telegram, now = Date.now }) {
  const limiter = new RateLimiter(config.maxRequestsPerMinute);

  async function authenticateDesktop(request) {
    const verified = verifyDesktopRequest({
      headers: request.headers,
      method: request.method,
      pathname: request.pathname,
      body: request.body,
      secret: config.desktopSharedSecret,
      nowMs: now(),
      maxClockSkewSeconds: config.maxClockSkewSeconds,
    });
    if (!verified.ok) return { error: jsonResponse(401, { error: "unauthorized" }) };
    const replayId = sha256Hex(`${verified.timestampMs}:${verified.nonce}`);
    const replay = await repository.createOnce("relay_nonces", replayId, { createdAtMs: now(), expiresAt: new Date(now() + 86_400_000) });
    if (!replay) return { error: jsonResponse(409, { error: "replayed_request" }) };
    return verified;
  }

  return async function handle(request) {
    if (request.method === "GET" && request.pathname === "/healthz") {
      return jsonResponse(200, { ok: true, service: "investa-cloud-relay", liveOrderEnabled: false });
    }

    if (request.tooLarge || Buffer.byteLength(request.body) > config.maxBodyBytes) return jsonResponse(413, { error: "request_too_large" });
    if (!limiter.allow(request.remoteAddress ?? "unknown", now())) return jsonResponse(429, { error: "rate_limited" });

    if (request.method === "POST" && request.pathname === "/telegram/webhook") {
      if (!verifyTelegramSecret(request.headers["x-telegram-bot-api-secret-token"], config.telegramWebhookSecret)) {
        return jsonResponse(401, { error: "unauthorized" });
      }
      let update;
      try { update = JSON.parse(request.body); } catch { return jsonResponse(400, { error: "invalid_json" }); }
      const message = update?.message;
      const userId = String(message?.from?.id ?? "");
      const chatId = String(message?.chat?.id ?? "");
      const instruction = typeof message?.text === "string" ? message.text.trim() : "";
      if (!Number.isSafeInteger(update?.update_id) || !config.allowedTelegramUserIds.has(userId)) {
        return jsonResponse(403, { error: "forbidden" });
      }
      if (!/^-?\d{1,31}$/.test(chatId) || instruction.length === 0 || [...instruction].length > 4_000) {
        return jsonResponse(400, { error: "invalid_instruction" });
      }
      if (containsSecretMarker(instruction)) {
        return jsonResponse(400, { error: "sensitive_instruction" });
      }
      const jobId = `telegram-${update.update_id}`;
      const createdAtMs = now();
      const expiresAtMs = createdAtMs + config.jobRetentionHours * 3_600_000;
      const created = await repository.createOnce("relay_jobs", jobId, {
        jobId,
        sourceRequestId: `update:${update.update_id}`,
        sourceUserId: userId,
        sourceChatId: chatId,
        instruction,
        status: "queued",
        createdAtMs,
        updatedAtMs: createdAtMs,
        expiresAtMs,
        expiresAt: new Date(expiresAtMs),
      });
      if (created) await telegram.sendMessage(chatId, "Investa가 지시를 안전한 작업 큐에 등록했습니다. 위험 작업은 PC에서 다시 승인해야 합니다.");
      return jsonResponse(200, { ok: true, duplicate: !created });
    }

    if (request.method === "POST" && request.pathname === "/v1/jobs/pull") {
      const auth = await authenticateDesktop(request);
      if (auth.error) return auth.error;
      let payload;
      try { payload = JSON.parse(request.body); } catch { return jsonResponse(400, { error: "invalid_json" }); }
      if (!validDeviceId(payload.deviceId)) return jsonResponse(400, { error: "invalid_device_id" });
      const job = await repository.claimOldest(payload.deviceId, now(), config.leaseSeconds);
      if (!job) return jsonResponse(200, { job: null });
      return jsonResponse(200, { job: {
        jobId: job.jobId,
        sourceRequestId: job.sourceRequestId,
        sourceUserId: job.sourceUserId,
        sourceChatId: job.sourceChatId,
        instruction: job.instruction,
        receivedAtMs: job.createdAtMs,
        leaseExpiresAtMs: job.leaseExpiresAtMs,
      } });
    }

    const resultMatch = request.pathname.match(/^\/v1\/jobs\/([A-Za-z0-9_-]{1,128})\/result$/);
    if (request.method === "POST" && resultMatch) {
      const auth = await authenticateDesktop(request);
      if (auth.error) return auth.error;
      let payload;
      try { payload = JSON.parse(request.body); } catch { return jsonResponse(400, { error: "invalid_json" }); }
      if (!validDeviceId(payload.deviceId) || !["accepted", "awaiting_local_approval", "approved", "rejected", "cancelled", "completed", "failed"].includes(payload.status) || typeof payload.resultText !== "string") {
        return jsonResponse(400, { error: "invalid_result" });
      }
      if ([...payload.resultText].length > 12_000 || containsSecretMarker(payload.resultText)) {
        return jsonResponse(400, { error: "sensitive_or_oversized_result" });
      }
      const jobId = resultMatch[1];
      const current = await repository.get("relay_jobs", jobId);
      if (!current || current.status !== "leased" || current.leasedBy !== payload.deviceId || Number(current.leaseExpiresAtMs ?? 0) < now()) {
        return jsonResponse(409, { error: "lease_not_owned" });
      }
      const statusText = payload.status === "awaiting_local_approval" ? "PC의 로컬 승인을 기다립니다." : payload.resultText;
      await telegram.sendMessage(current.sourceChatId, statusText || `Investa 작업 상태: ${payload.status}`);
      await repository.patch("relay_jobs", jobId, {
        status: payload.status,
        localJobId: String(payload.localJobId ?? "").slice(0, 128),
        resultText: payload.resultText.slice(0, 12_000),
        leasedBy: payload.deviceId,
        updatedAtMs: now(),
      }, current.updateTime);
      return jsonResponse(200, { ok: true });
    }

    return jsonResponse(404, { error: "not_found" });
  };
}
