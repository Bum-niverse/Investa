export const SHADOW_SOAK_DURATION_MS = 86_400_000;
export const SHADOW_SOAK_SAMPLE_INTERVAL_MS = 60_000;
export const SHADOW_SOAK_MAX_SAMPLES = 2_000;

export type ShadowSoakSample = {
  observedAtMs: number;
  sourceObservedAtMs?: number | null;
  memoryBytes: number;
  timerCount: number;
  sqliteBytes: number;
  candidateKey?: string | null;
  providerHealthy: boolean;
  restarted: boolean;
  reconciliationPassed: boolean;
};

export type ShadowSoakSession = {
  schema: "investa.shadow-soak-session.v1";
  runId: string;
  startedAtMs: number;
  lastBootId: string;
  samples: ShadowSoakSample[];
};

export function parseShadowSoakSession(raw: string | null): ShadowSoakSession | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<ShadowSoakSession>;
    if (value.schema !== "investa.shadow-soak-session.v1" || typeof value.runId !== "string" || !/^shadow-soak-[0-9]+$/.test(value.runId) || typeof value.startedAtMs !== "number" || typeof value.lastBootId !== "string" || !Array.isArray(value.samples) || value.samples.length > SHADOW_SOAK_MAX_SAMPLES) return null;
    if (value.samples.some((sample) => typeof sample?.observedAtMs !== "number" || typeof sample?.memoryBytes !== "number" || typeof sample?.timerCount !== "number" || typeof sample?.sqliteBytes !== "number" || typeof sample?.providerHealthy !== "boolean" || typeof sample?.restarted !== "boolean" || typeof sample?.reconciliationPassed !== "boolean")) return null;
    return value as ShadowSoakSession;
  } catch {
    return null;
  }
}

export function appendShadowSoakSample(session: ShadowSoakSession, sample: ShadowSoakSample, bootId: string): ShadowSoakSession {
  const previous = session.samples[session.samples.length - 1];
  if (previous && sample.observedAtMs <= previous.observedAtMs) throw new Error("내구 검사 표본 시각이 이전 표본보다 늦지 않습니다.");
  if (session.samples.length >= SHADOW_SOAK_MAX_SAMPLES) throw new Error("내구 검사 표본 상한을 초과했습니다.");
  return { ...session, lastBootId: bootId, samples: [...session.samples, sample] };
}

export function keepOnlyNewCandidateKey(session: ShadowSoakSession, sample: ShadowSoakSample): ShadowSoakSample {
  if (!sample.candidateKey || !session.samples.some((existing) => existing.candidateKey === sample.candidateKey)) return sample;
  return { ...sample, candidateKey: null };
}

export function shadowSoakElapsedMs(session: ShadowSoakSession, nowMs: number): number {
  return Math.max(0, nowMs - session.startedAtMs);
}

export function shadowSoakReadyToFinalize(session: ShadowSoakSession, nowMs: number): boolean {
  return session.samples.length >= 2 && shadowSoakElapsedMs(session, nowMs) >= SHADOW_SOAK_DURATION_MS;
}
