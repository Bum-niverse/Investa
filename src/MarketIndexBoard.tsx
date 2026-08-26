import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  formatMarketChange,
  formatMarketValue,
  getMarketQuote,
  type MarketIndexCode,
  type MarketIndexSnapshot,
} from "./marketIndices";

const INDEX_CODES: MarketIndexCode[] = ["KOSPI", "KOSDAQ", "NASDAQ"];

const QUOTE_STATE_LABELS = {
  live: "정상",
  delayed: "지연",
  closed: "장 마감",
  unavailable: "미연결",
} as const;

type MarketIndexBoardProps = {
  snapshot: MarketIndexSnapshot;
  compact?: boolean;
};

export function MarketIndexBoard({ snapshot, compact = false }: MarketIndexBoardProps) {
  const [isDetailOpen, setIsDetailOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!isDetailOpen) return;

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    closeButtonRef.current?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsDetailOpen(false);
        window.requestAnimationFrame(() => triggerRef.current?.focus());
      }
      if (event.key === "Tab") {
        event.preventDefault();
        closeButtonRef.current?.focus();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [isDetailOpen]);

  const closeDetail = () => {
    setIsDetailOpen(false);
    window.requestAnimationFrame(() => triggerRef.current?.focus());
  };

  return (
    <>
      <section className={`market-index-board ${compact ? "is-compact" : ""}`} aria-label="시장 지수 시세">
        <button
          ref={triggerRef}
          className="market-index-board-trigger"
          type="button"
          aria-label="증시 지표 상세 보기"
          title={`${snapshot.message} · 눌러서 자세히 보기`}
          onClick={() => setIsDetailOpen(true)}
        />
        <header>
          <b>MARKET</b>
          <span>{snapshot.provider ?? "FEED WAIT"}</span>
        </header>
        <div className="market-index-rows">
          {INDEX_CODES.map((code) => {
            const quote = getMarketQuote(snapshot, code);
            const direction = quote.changePercent === null
              ? "is-unavailable"
              : quote.changePercent > 0
                ? "is-up"
                : quote.changePercent < 0
                  ? "is-down"
                  : "is-flat";
            return (
              <div className={`market-index-row ${direction}`} key={code}>
                <strong>{code}</strong>
                <span>{formatMarketValue(quote.value)}</span>
                <em>{formatMarketChange(quote.changePercent)}</em>
              </div>
            );
          })}
        </div>
        {!compact && <footer>{snapshot.message}</footer>}
      </section>
      {isDetailOpen && createPortal(
        <div className="market-index-dialog-backdrop" role="presentation" onMouseDown={(event) => {
          if (event.target === event.currentTarget) closeDetail();
        }}>
          <section className="market-index-dialog" role="dialog" aria-modal="true" aria-labelledby="market-index-dialog-title">
            <header>
              <div>
                <span>MARKET STATUS</span>
                <h2 id="market-index-dialog-title">증시 지표</h2>
              </div>
              <button ref={closeButtonRef} type="button" onClick={closeDetail} aria-label="증시 지표 닫기">닫기</button>
            </header>
            <div className="market-index-dialog-meta">
              <span>공급자 <strong>{snapshot.provider ?? "연결 대기"}</strong></span>
              <span>갱신 <strong>{formatObservedTime(snapshot.fetchedAt)}</strong></span>
            </div>
            <div className="market-index-dialog-quotes">
              {INDEX_CODES.map((code) => {
                const quote = getMarketQuote(snapshot, code);
                return (
                  <article className={`market-index-dialog-quote is-${quote.state}`} key={code}>
                    <div>
                      <strong>{code}</strong>
                      <span>{QUOTE_STATE_LABELS[quote.state]}</span>
                    </div>
                    <p>{formatMarketValue(quote.value)}</p>
                    <dl>
                      <div><dt>등락률</dt><dd>{quote.changePercent === null ? "공급자 미제공" : formatMarketChange(quote.changePercent)}</dd></div>
                      <div><dt>관측 시각</dt><dd>{formatObservedTime(quote.observedAt)}</dd></div>
                    </dl>
                  </article>
                );
              })}
            </div>
            <footer>{snapshot.message}</footer>
          </section>
        </div>,
        document.body,
      )}
    </>
  );
}

function formatObservedTime(value: string | null) {
  if (!value) return "확인 불가";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "확인 불가";
  return new Intl.DateTimeFormat("ko-KR", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(date);
}
