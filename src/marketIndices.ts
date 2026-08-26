export type MarketIndexCode = "KOSPI" | "KOSDAQ" | "NASDAQ";
export type MarketQuoteState = "live" | "delayed" | "closed" | "unavailable";

export type MarketIndexQuote = {
  code: MarketIndexCode;
  value: number | null;
  changePercent: number | null;
  observedAt: string | null;
  state: MarketQuoteState;
};

export type MarketIndexSnapshot = {
  provider: string | null;
  fetchedAt: string | null;
  refreshAfterMs: number;
  message: string;
  quotes: MarketIndexQuote[];
};

export const EMPTY_MARKET_INDEX_SNAPSHOT: MarketIndexSnapshot = {
  provider: null,
  fetchedAt: null,
  refreshAfterMs: 15_000,
  message: "공식 시세 공급자 연결 대기",
  quotes: (["KOSPI", "KOSDAQ", "NASDAQ"] as const).map((code) => ({
    code,
    value: null,
    changePercent: null,
    observedAt: null,
    state: "unavailable",
  })),
};

export function formatMarketValue(value: number | null) {
  return value === null
    ? "---.--"
    : new Intl.NumberFormat("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(value);
}

export function formatMarketChange(changePercent: number | null) {
  if (changePercent === null) return "대기";
  const sign = changePercent > 0 ? "+" : "";
  return `${sign}${changePercent.toFixed(2)}%`;
}

export function getMarketQuote(snapshot: MarketIndexSnapshot, code: MarketIndexCode) {
  return snapshot.quotes.find((quote) => quote.code === code)
    ?? EMPTY_MARKET_INDEX_SNAPSHOT.quotes.find((quote) => quote.code === code)!;
}
