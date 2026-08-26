export type IntegrationKind = "broker" | "exchange" | "ai";
export type IntegrationCapability = "market-data" | "account-read" | "paper-order" | "live-order" | "analysis";
export type IntegrationSupport = "built-in" | "partial" | "adapter-required";

export type IntegrationDefinition = {
  id: string;
  name: string;
  kind: IntegrationKind;
  markets: Array<"kr" | "us" | "coin" | "futures">;
  capabilities: IntegrationCapability[];
  support: IntegrationSupport;
  summary: string;
};

export const INTEGRATION_CATALOG: IntegrationDefinition[] = [
  { id: "toss", name: "토스증권", kind: "broker", markets: ["kr", "us"], capabilities: ["market-data", "account-read"], support: "built-in", summary: "국장·미장 시세, 계좌·보유자산 조회" },
  { id: "kis-paper", name: "한국투자증권", kind: "broker", markets: ["kr"], capabilities: ["account-read", "paper-order"], support: "built-in", summary: "국내주식 모의계좌·모의주문" },
  { id: "upbit", name: "Upbit", kind: "exchange", markets: ["coin"], capabilities: ["market-data", "account-read"], support: "built-in", summary: "코인 공개 시세·캔들, 개인계좌 읽기 전용 조회" },
  { id: "binance", name: "Binance", kind: "exchange", markets: ["coin", "futures"], capabilities: ["market-data", "account-read"], support: "built-in", summary: "현물·USDⓈ-M·COIN-M 공개 시세와 계좌·포지션 읽기 전용 조회" },
  { id: "codex", name: "Codex", kind: "ai", markets: ["kr", "us", "coin", "futures"], capabilities: ["analysis"], support: "built-in", summary: "사용자의 Codex CLI 로그인으로 읽기 전용 분석" },
  { id: "custom", name: "기타 증권사·거래소·AI", kind: "broker", markets: ["kr", "us", "coin", "futures"], capabilities: [], support: "adapter-required", summary: "공급자의 공식 API별 어댑터 구현 필요" },
];

export const supportLabel: Record<IntegrationSupport, string> = {
  "built-in": "지원",
  partial: "일부 지원",
  "adapter-required": "어댑터 필요",
};
