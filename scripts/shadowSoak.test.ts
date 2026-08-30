import assert from "node:assert/strict";
import test from "node:test";
import { appendShadowSoakSample, keepOnlyNewCandidateKey, parseShadowSoakSession, shadowSoakReadyToFinalize, type ShadowSoakSample, type ShadowSoakSession } from "../src/shadowSoak.ts";

const sample = (observedAtMs: number): ShadowSoakSample => ({ observedAtMs, sourceObservedAtMs: observedAtMs, memoryBytes: 100, timerCount: 1, sqliteBytes: 200, candidateKey: null, providerHealthy: true, restarted: false, reconciliationPassed: true });
const session = (): ShadowSoakSession => ({ schema: "investa.shadow-soak-session.v1", runId: "shadow-soak-1", startedAtMs: 1, lastBootId: "boot-a", samples: [] });

test("내구 검사 세션은 잘못된 저장값과 표본 역순을 거부한다", () => {
  assert.equal(parseShadowSoakSession('{"schema":"wrong"}'), null);
  const first = appendShadowSoakSample(session(), sample(10), "boot-a");
  assert.throws(() => appendShadowSoakSample(first, sample(10), "boot-a"));
});

test("실제 24시간과 두 표본을 모두 충족해야 종료 가능하다", () => {
  const first = appendShadowSoakSample(session(), sample(1), "boot-a");
  assert.equal(shadowSoakReadyToFinalize(first, 86_400_001), false);
  const second = appendShadowSoakSample(first, sample(86_400_001), "boot-b");
  assert.equal(second.samples[1].restarted, false);
  assert.equal(shadowSoakReadyToFinalize(second, 86_400_001), true);
});

test("같은 최근 후보를 매분 중복 생성으로 오판하지 않는다", () => {
  const firstCandidate = { ...sample(1), candidateKey: "candidate-1" };
  const first = appendShadowSoakSample(session(), firstCandidate, "boot-a");
  assert.equal(keepOnlyNewCandidateKey(first, { ...sample(2), candidateKey: "candidate-1" }).candidateKey, null);
  assert.equal(keepOnlyNewCandidateKey(first, { ...sample(2), candidateKey: "candidate-2" }).candidateKey, "candidate-2");
});
