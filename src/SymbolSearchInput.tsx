import { invoke } from "@tauri-apps/api/core";
import { type RefObject, useEffect, useId, useMemo, useRef, useState } from "react";

export type SearchMarket = "kr" | "us" | "coin";
export type SymbolSearchResult = {
  symbol: string;
  name: string;
  market: string;
  currency: string;
  securityType: string;
};

const BUILTIN_SYMBOLS: SymbolSearchResult[] = [
  { symbol: "000660", name: "SK하이닉스", market: "KOSPI", currency: "KRW", securityType: "STOCK" },
  { symbol: "005930", name: "삼성전자", market: "KOSPI", currency: "KRW", securityType: "STOCK" },
  { symbol: "012450", name: "한화에어로스페이스", market: "KOSPI", currency: "KRW", securityType: "STOCK" },
  { symbol: "AAPL", name: "Apple · 애플", market: "NASDAQ", currency: "USD", securityType: "STOCK" },
  { symbol: "MSFT", name: "Microsoft · 마이크로소프트", market: "NASDAQ", currency: "USD", securityType: "STOCK" },
  { symbol: "GOOGL", name: "Alphabet · 구글", market: "NASDAQ", currency: "USD", securityType: "STOCK" },
  { symbol: "TSLA", name: "Tesla · 테슬라", market: "NASDAQ", currency: "USD", securityType: "STOCK" },
  { symbol: "NVDA", name: "NVIDIA · 엔비디아", market: "NASDAQ", currency: "USD", securityType: "STOCK" },
  { symbol: "AMZN", name: "Amazon · 아마존", market: "NASDAQ", currency: "USD", securityType: "STOCK" },
  { symbol: "META", name: "Meta · 메타", market: "NASDAQ", currency: "USD", securityType: "STOCK" },
  { symbol: "KRW-BTC", name: "비트코인", market: "UPBIT", currency: "KRW", securityType: "CRYPTO" },
  { symbol: "KRW-ETH", name: "이더리움", market: "UPBIT", currency: "KRW", securityType: "CRYPTO" },
  { symbol: "KRW-XRP", name: "리플", market: "UPBIT", currency: "KRW", securityType: "CRYPTO" },
];

const normalize = (value: string) => value.normalize("NFKC").replace(/[^\p{L}\p{N}]/gu, "").toUpperCase();
const belongsToMarket = (item: SymbolSearchResult, market: SearchMarket) => market === "coin"
  ? item.market === "UPBIT"
  : market === "kr" ? item.currency === "KRW" && item.market !== "UPBIT" : item.currency === "USD";
const localMatches = (market: SearchMarket, query: string) => {
  const needle = normalize(query);
  if (!needle) return [];
  return BUILTIN_SYMBOLS.filter((item) => belongsToMarket(item, market) && normalize(`${item.name}${item.symbol}`).includes(needle)).slice(0, 8);
};

export function SymbolSearchInput({ id, market, value, placeholder, inputRef, onChange, onSelect }: {
  id?: string;
  market: SearchMarket;
  value: string;
  placeholder: string;
  inputRef?: RefObject<HTMLInputElement | null>;
  onChange: (value: string) => void;
  onSelect: (result: SymbolSearchResult) => void;
}) {
  const listboxId = useId();
  const requestSequence = useRef(0);
  const [remoteResults, setRemoteResults] = useState<SymbolSearchResult[]>([]);
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const localResults = useMemo(() => localMatches(market, value), [market, value]);
  const results = useMemo(() => {
    const merged = [...localResults, ...remoteResults];
    return merged.filter((item, index) => merged.findIndex((candidate) => candidate.symbol === item.symbol) === index).slice(0, 8);
  }, [localResults, remoteResults]);

  useEffect(() => {
    setRemoteResults([]);
    setActiveIndex(-1);
    const query = value.trim();
    if (market === "coin" || query.length < 1) { setLoading(false); return; }
    const sequence = ++requestSequence.current;
    const timer = window.setTimeout(() => {
      setLoading(true);
      void invoke<SymbolSearchResult[]>("toss_search_stocks", { request: { market, query } })
        .then((items) => { if (requestSequence.current === sequence) setRemoteResults(items); })
        .catch(() => { if (requestSequence.current === sequence) setRemoteResults([]); })
        .finally(() => { if (requestSequence.current === sequence) setLoading(false); });
    }, 280);
    return () => window.clearTimeout(timer);
  }, [market, value]);

  const select = (item: SymbolSearchResult) => {
    onSelect(item);
    setOpen(false);
    setActiveIndex(-1);
  };

  return <div className="symbol-search">
    <input
      id={id}
      ref={inputRef}
      role="combobox"
      aria-autocomplete="list"
      aria-expanded={open && (loading || results.length > 0)}
      aria-controls={listboxId}
      aria-activedescendant={activeIndex >= 0 ? `${listboxId}-${activeIndex}` : undefined}
      value={value}
      onChange={(event) => { onChange(event.currentTarget.value); setOpen(true); }}
      onFocus={() => setOpen(true)}
      onBlur={() => window.setTimeout(() => setOpen(false), 120)}
      onKeyDown={(event) => {
        if (event.key === "ArrowDown" && results.length) { event.preventDefault(); setOpen(true); setActiveIndex((current) => Math.min(results.length - 1, current + 1)); }
        if (event.key === "ArrowUp" && results.length) { event.preventDefault(); setActiveIndex((current) => Math.max(0, current - 1)); }
        if (event.key === "Enter" && open && activeIndex >= 0 && results[activeIndex]) { event.preventDefault(); select(results[activeIndex]); }
        if (event.key === "Escape") { setOpen(false); setActiveIndex(-1); }
      }}
      placeholder={placeholder}
      spellCheck={false}
      autoComplete="off"
    />
    {open && value.trim() && (loading || results.length > 0) && <div className="symbol-search-results" id={listboxId} role="listbox" aria-label="종목 검색 결과">
      {results.map((item, index) => <button
        id={`${listboxId}-${index}`}
        type="button"
        role="option"
        aria-selected={activeIndex === index}
        className={activeIndex === index ? "is-active" : ""}
        key={`${item.market}-${item.symbol}`}
        onMouseDown={(event) => event.preventDefault()}
        onClick={() => select(item)}
      ><span><strong>{item.name}</strong><small>{item.market} · {item.securityType}</small></span><b>{item.symbol}</b></button>)}
      {loading && <p role="status">공식 종목 목록 확인 중…</p>}
    </div>}
  </div>;
}
