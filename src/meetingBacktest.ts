export type MeetingResearchSignal =
  | { type: "moving_average_cross"; fastWindow: number; slowWindow: number; direction: "above" | "below" }
  | { type: "price_channel_breakout"; lookback: number; direction: "above" | "below" }
  | { type: "mean_reversion"; window: number; deviationBps: number; direction: "above" | "below" }
  | { type: "volatility_expansion"; atrWindow: number; breakoutWindow: number; minimumExpansionBps: number; direction: "above" | "below" };

export type MeetingBacktestReport = {
  traceId: string;
  request: string;
  evidence: Array<{
    evidenceId: string;
    kind: "local_analysis";
    sourceUrl: string;
    summary: string;
  }>;
  strategyCandidate: {
    schemaVersion: "1";
    strategyId: string;
    name: string;
    market: "korea" | "united_states" | "crypto";
    symbol: string;
    currency: string;
    hypothesis: string;
    sourceEvidenceIds: string[];
    entrySignal: MeetingResearchSignal;
    exitSignal: MeetingResearchSignal;
    limitations: string[];
    unknowns: string[];
  };
};

type BuildMeetingBacktestReportRequest = {
  workflowJobId: string;
  topic: string;
  analysisRecordId: string;
  symbol: string;
  strategy: string;
  market: string;
  currency: string;
};

const safeIdentifier = (value: string, fallback: string) => {
  const normalized = value.replace(/[^A-Za-z0-9_-]/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "");
  return (normalized || fallback).slice(0, 64);
};

const positiveInteger = (value: string | undefined) => {
  if (!value) return null;
  const parsed = Number.parseInt(value, 10);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
};

export function parseMeetingStrategy(strategy: string): { entry: MeetingResearchSignal; exit: MeetingResearchSignal; label: string } {
  const normalized = strategy.trim().replace(/\s+/g, " ");

  const movingAverage = normalized.match(/(\d{1,4})\s*[\/-]\s*(\d{1,4})\s*(?:일|봉)?\s*(?:이동평균|이평|MA)\s*(?:선\s*)?(?:교차|크로스)/i);
  if (movingAverage) {
    const first = positiveInteger(movingAverage[1]);
    const second = positiveInteger(movingAverage[2]);
    if (first && second && first !== second) {
      const fastWindow = Math.min(first, second);
      const slowWindow = Math.max(first, second);
      return {
        entry: { type: "moving_average_cross", fastWindow, slowWindow, direction: "above" },
        exit: { type: "moving_average_cross", fastWindow, slowWindow, direction: "below" },
        label: `${fastWindow}/${slowWindow} 이동평균 교차`,
      };
    }
  }

  const priceChannel = normalized.match(/(\d{1,4})\s*(?:봉|일)?\s*(?:가격\s*)?채널\s*돌파/i);
  if (priceChannel) {
    const lookback = positiveInteger(priceChannel[1]);
    if (lookback && lookback >= 2) {
      return {
        entry: { type: "price_channel_breakout", lookback, direction: "above" },
        exit: { type: "price_channel_breakout", lookback, direction: "below" },
        label: `${lookback}봉 가격 채널 돌파`,
      };
    }
  }

  const meanReversion = normalized.match(/(\d{1,4})\s*(?:봉|일)?\s*(?:평균\s*회귀|이격\s*회귀).*?(\d{1,4})\s*bp/i);
  if (meanReversion) {
    const window = positiveInteger(meanReversion[1]);
    const deviationBps = positiveInteger(meanReversion[2]);
    if (window && window >= 2 && deviationBps && deviationBps < 10_000) {
      return {
        entry: { type: "mean_reversion", window, deviationBps, direction: "below" },
        exit: { type: "mean_reversion", window, deviationBps, direction: "above" },
        label: `${window}봉 평균회귀 ${deviationBps}bp`,
      };
    }
  }

  const volatility = normalized.match(/ATR\s*(\d{1,4}).*?(?:돌파|breakout)\s*(\d{1,4}).*?(\d{1,6})\s*bp/i);
  if (volatility) {
    const atrWindow = positiveInteger(volatility[1]);
    const breakoutWindow = positiveInteger(volatility[2]);
    const minimumExpansionBps = positiveInteger(volatility[3]);
    if (atrWindow && atrWindow >= 2 && breakoutWindow && breakoutWindow >= 2 && minimumExpansionBps && minimumExpansionBps <= 100_000) {
      return {
        entry: { type: "volatility_expansion", atrWindow, breakoutWindow, minimumExpansionBps, direction: "above" },
        exit: { type: "volatility_expansion", atrWindow, breakoutWindow, minimumExpansionBps, direction: "below" },
        label: `ATR ${atrWindow} · ${breakoutWindow}봉 돌파 · ${minimumExpansionBps}bp`,
      };
    }
  }

  throw new Error("지원 전략 형식이 아닙니다. 예: 5/20 이동평균 교차, 20봉 가격 채널 돌파, 20봉 평균회귀 200bp, ATR 14 돌파 20 12500bp");
}

export function buildMeetingBacktestReport(request: BuildMeetingBacktestReportRequest): MeetingBacktestReport {
  const parsed = parseMeetingStrategy(request.strategy);
  const market = request.market === "korea" || request.market === "united_states" || request.market === "crypto"
    ? request.market
    : null;
  if (!market) throw new Error("현재 회의 자동 백테스트는 국장·미장·코인 현물만 지원합니다.");
  if (!/^[A-Z0-9.-]{1,32}$/.test(request.symbol)) throw new Error("백테스트할 단일 종목 코드가 올바르지 않습니다.");
  if (!/^[A-Z]{3}$/.test(request.currency)) throw new Error("백테스트 통화가 올바르지 않습니다.");

  const evidenceId = safeIdentifier(`meeting-${request.workflowJobId}`, "meeting-analysis");
  const traceId = safeIdentifier(`meeting-${request.workflowJobId}`, "meeting-trace");
  const strategyId = safeIdentifier(`${traceId}-${parsed.entry.type}`, "meeting-strategy");
  return {
    traceId,
    request: request.topic,
    evidence: [{
      evidenceId,
      kind: "local_analysis",
      sourceUrl: `investa://analysis/${request.analysisRecordId}`,
      summary: "부서별 근거 검증과 본부장 종합을 마친 로컬 회의 분석 기록입니다.",
    }],
    strategyCandidate: {
      schemaVersion: "1",
      strategyId,
      name: parsed.label,
      market,
      symbol: request.symbol,
      currency: request.currency,
      hypothesis: `${request.topic} 회의에서 제안된 ${parsed.label} 규칙을 시점 정합 완료 봉으로 탐색 검증합니다.`,
      sourceEvidenceIds: [evidenceId],
      entrySignal: parsed.entry,
      exitSignal: parsed.exit,
      limitations: [
        "최대 200개 최신 완료 봉을 사용하는 탐색 백테스트이며 미래 수익을 보장하지 않습니다.",
        "백테스트 통과와 현재 진입 신호는 별도이며, 신호가 없으면 섀도우 감시만 유지합니다.",
      ],
      unknowns: [],
    },
  };
}
