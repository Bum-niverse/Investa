import test from "node:test";
import assert from "node:assert/strict";
import { signRequest, verifyDesktopRequest } from "../src/security.mjs";

const secret = "a-secure-shared-secret-that-is-long-enough";

test("desktop HMAC accepts an exact fresh request", () => {
  const request = { timestamp: "1787600000000", nonce: "nonce_1234567890abcdef", method: "POST", pathname: "/v1/jobs/pull", body: '{"deviceId":"pc-1"}' };
  const signature = signRequest(secret, request);
  assert.equal(signature, "1858eb9ef2222313e500b849471b540fcb5c9c85eb7cc484c1a2170b9c0c5b2b");
  assert.equal(verifyDesktopRequest({
    headers: { "x-investa-timestamp": request.timestamp, "x-investa-nonce": request.nonce, "x-investa-signature": signature },
    method: request.method,
    pathname: request.pathname,
    body: request.body,
    secret,
    nowMs: Number(request.timestamp),
    maxClockSkewSeconds: 300,
  }).ok, true);
});

test("desktop HMAC rejects body mutation and stale timestamps", () => {
  const request = { timestamp: "1787600000000", nonce: "nonce_1234567890abcdef", method: "POST", pathname: "/v1/jobs/pull", body: "{}" };
  const headers = {
    "x-investa-timestamp": request.timestamp,
    "x-investa-nonce": request.nonce,
    "x-investa-signature": signRequest(secret, request),
  };
  assert.equal(verifyDesktopRequest({ headers, ...request, body: '{"changed":true}', secret, nowMs: Number(request.timestamp), maxClockSkewSeconds: 300 }).ok, false);
  assert.equal(verifyDesktopRequest({ headers, ...request, secret, nowMs: Number(request.timestamp) + 301_000, maxClockSkewSeconds: 300 }).reason, "timestamp_out_of_range");
});
