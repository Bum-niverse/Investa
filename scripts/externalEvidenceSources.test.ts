import assert from "node:assert/strict";
import test from "node:test";
import { markCitedEvidenceSources, naverNewsEvidenceSource, telegramEvidenceSource } from "../src/externalEvidenceSources.ts";

test("네이버 뉴스는 원문 언론사 도메인·제목·네이버 제공 링크를 분리한다", () => {
  const source = naverNewsEvidenceSource("naver-news-1", {
    title: "한화 인적분할 관련 기사",
    originalLink: "https://www.example-news.co.kr/article/1",
    link: "https://n.news.naver.com/article/001/1",
    publishedAt: "Fri, 04 Sep 2026 09:00:00 +0900",
  }, 100);
  assert.equal(source.sourceName, "example-news.co.kr");
  assert.equal(source.sourceUrl, "https://www.example-news.co.kr/article/1");
  assert.equal(source.platformUrl, "https://n.news.naver.com/article/001/1");
  assert.equal(source.cited, false);
});

test("Telegram 공개 채널만 안전한 메시지 URL을 만들고 비공개 채널은 URL을 만들지 않는다", () => {
  const publicSource = telegramEvidenceSource("telegram-1", {
    sourceTitle: "투자 뉴스",
    sourceUsername: "market_news_kr",
    messageId: 77,
    postedAtMs: 1_700_000_000_000,
    text: "한화 관련 공지",
  }, 1_700_000_000_100);
  const privateSource = telegramEvidenceSource("telegram-2", {
    sourceTitle: "비공개 채널",
    sourceUsername: null,
    messageId: 88,
    postedAtMs: 1_700_000_000_000,
    text: "내부 전달",
  }, 1_700_000_000_100);
  assert.equal(publicSource.sourceUrl, "https://t.me/market_news_kr/77");
  assert.equal(privateSource.sourceUrl, null);
});

test("조회한 근거와 실제 보고서 인용 근거를 evidenceId로 구분한다", () => {
  const sources = [
    naverNewsEvidenceSource("news-1", { title: "기사 1", originalLink: "https://a.example/1", link: "", publishedAt: "today" }, 1),
    naverNewsEvidenceSource("news-2", { title: "기사 2", originalLink: "https://b.example/2", link: "", publishedAt: "today" }, 2),
  ];
  const marked = markCitedEvidenceSources(sources, ["news-2"]);
  assert.equal(marked.find((item) => item.evidenceId === "news-1")?.cited, false);
  assert.equal(marked.find((item) => item.evidenceId === "news-2")?.cited, true);
});
