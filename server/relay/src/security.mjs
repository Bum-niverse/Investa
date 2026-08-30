import crypto from "node:crypto";

export function sha256Hex(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

export function safeEqual(left, right) {
  const leftBuffer = Buffer.from(String(left));
  const rightBuffer = Buffer.from(String(right));
  return leftBuffer.length === rightBuffer.length && crypto.timingSafeEqual(leftBuffer, rightBuffer);
}

export function canonicalRequest({ timestamp, nonce, method, pathname, body }) {
  return `${timestamp}\n${nonce}\n${method.toUpperCase()}\n${pathname}\n${sha256Hex(body)}`;
}

export function signRequest(secret, request) {
  return crypto.createHmac("sha256", secret).update(canonicalRequest(request)).digest("hex");
}

export function verifyDesktopRequest({ headers, method, pathname, body, secret, nowMs, maxClockSkewSeconds }) {
  const timestamp = headers["x-investa-timestamp"];
  const nonce = headers["x-investa-nonce"];
  const signature = headers["x-investa-signature"];
  if (!timestamp || !nonce || !signature || !/^\d{10,13}$/.test(timestamp) || !/^[A-Za-z0-9_-]{16,128}$/.test(nonce)) {
    return { ok: false, reason: "missing_or_invalid_signature_headers" };
  }
  const timestampMs = timestamp.length === 10 ? Number(timestamp) * 1_000 : Number(timestamp);
  if (!Number.isSafeInteger(timestampMs) || Math.abs(nowMs - timestampMs) > maxClockSkewSeconds * 1_000) {
    return { ok: false, reason: "timestamp_out_of_range" };
  }
  const expected = signRequest(secret, { timestamp, nonce, method, pathname, body });
  if (!safeEqual(expected, signature)) return { ok: false, reason: "signature_mismatch" };
  return { ok: true, timestampMs, nonce };
}

export function verifyTelegramSecret(actual, expected) {
  return Boolean(actual) && safeEqual(actual, expected);
}

const SENSITIVE_PATTERNS = [
  /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/i,
  /\b(?:ghp|github_pat|sk-proj|xox[baprs]|AIza)[A-Za-z0-9_-]{12,}\b/,
  /\b(?:api[_ -]?key|secret(?:[_ -]?key)?|access[_ -]?token|refresh[_ -]?token|password|비밀번호|시크릿)\s*[:=]\s*\S{8,}/i,
  /\b(?:authorization)\s*:\s*(?:bearer|basic)\s+\S+/i,
];

export function containsSecretMarker(value) {
  return typeof value === "string" && SENSITIVE_PATTERNS.some((pattern) => pattern.test(value));
}
