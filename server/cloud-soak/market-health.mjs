export const DEFAULT_MARKET_GAP_MS = 20_000;

export function createMarketStreamState() {
  return {
    messages: 0,
    reconnects: 0,
    errors: 0,
    transportTimeouts: 0,
    transportHeartbeats: 0,
    marketGapEvents: 0,
    firstMessageAtMs: null,
    lastMessageAtMs: null,
    lastTransportAtMs: null,
    maxGapMs: 0,
    marketGapOpen: false,
  };
}

export function recordMarketMessage(state, observedAtMs) {
  if (state.lastMessageAtMs !== null) {
    state.maxGapMs = Math.max(state.maxGapMs, observedAtMs - state.lastMessageAtMs);
  }
  state.messages += 1;
  state.firstMessageAtMs ??= observedAtMs;
  state.lastMessageAtMs = observedAtMs;
  state.lastTransportAtMs = observedAtMs;
  state.marketGapOpen = false;
}

export function recordTransportHeartbeat(state, observedAtMs) {
  state.transportHeartbeats += 1;
  state.lastTransportAtMs = observedAtMs;
}

export function observeMarketGap(state, observedAtMs, startedAtMs, thresholdMs = DEFAULT_MARKET_GAP_MS) {
  const reference = state.lastMessageAtMs ?? startedAtMs;
  if (observedAtMs - reference <= thresholdMs || state.marketGapOpen) return false;
  state.marketGapEvents += 1;
  state.marketGapOpen = true;
  return true;
}

export function isTransportTimedOut(state, observedAtMs, openedAtMs, timeoutMs) {
  return observedAtMs - (state.lastTransportAtMs ?? openedAtMs) > timeoutMs;
}

export function classifyUpbitPayload(payload) {
  if (typeof payload !== "string") return "market";
  try {
    const parsed = JSON.parse(payload);
    return parsed?.status === "UP" ? "heartbeat" : "market";
  } catch {
    return payload.trim() === "UP" ? "heartbeat" : "market";
  }
}

export function evaluateMarketStreams(definitions, states) {
  const issues = [];
  const warnings = [];
  for (const definition of definitions) {
    const state = states.get(definition.id);
    if (state.messages === 0) issues.push(`${definition.id}: 시세 메시지 미수신`);
    if (state.errors > 0) issues.push(`${definition.id}: WebSocket 오류 ${state.errors}회`);
    if (state.transportTimeouts > 0) {
      issues.push(`${definition.id}: 전송 생존 응답 제한 초과 ${state.transportTimeouts}회`);
    }
    if (state.maxGapMs > definition.marketGapThresholdMs) {
      const detail = `${definition.id}: 최대 시장 이벤트 간격 ${state.maxGapMs}ms`;
      if (definition.eventDriven) warnings.push(detail);
      else issues.push(detail);
    }
  }
  return { issues, warnings };
}
