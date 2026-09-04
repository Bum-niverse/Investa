import assert from "node:assert/strict";
import test from "node:test";
import { buildMeetingAnalysisContent, buildMeetingSynthesisPrompt, deduplicateMeetingEvidence, failedMeetingSynthesis } from "../src/meetingSynthesis.ts";

const evidence = (agentId: string, evidenceId: string, fill = "근거") => ({
  agentId,
  evidenceId,
  source: `https://example.com/${evidenceId}/${"s".repeat(400)}`,
  sourceRevision: "2026-09-04",
  observation: fill.repeat(500),
  observedAt: "2026-09-04T12:00:00+09:00",
});

test("직원별로 반복된 근거는 evidenceId 기준으로 합쳐 기여 직원을 보존한다", () => {
  const result = deduplicateMeetingEvidence({
    bull: [evidence("bull", "official-1"), evidence("bull", "official-2")],
    bear: [evidence("bear", "official-1"), evidence("bear", "official-3")],
  });
  assert.equal(result.occurrences, 4);
  assert.equal(result.evidence.length, 3);
  assert.deepEqual(result.evidence[0].agentIds, ["bull", "bear"]);
});

test("대규모 부서 보고도 본부장 안전 여유를 남긴 상한 안에서 단계형 입력으로 만든다", () => {
  const reports = Array.from({ length: 4 }, (_item, departmentIndex) => ({
    departmentId: `department-${departmentIndex}`,
    departmentName: `부서 ${departmentIndex}`,
    conclusion: "watch",
    confidencePercent: 80,
    summary: "부서 요약".repeat(300),
    roleFindings: Array.from({ length: 8 }, (_role, roleIndex) => ({
      agentId: `agent-${departmentIndex}-${roleIndex}`,
      role: "전문 분석가",
      finding: "직원 상세 소견".repeat(200),
      evidenceIds: Array.from({ length: 12 }, (_evidence, evidenceIndex) => `official-${evidenceIndex}`),
      counterevidence: ["반대 근거".repeat(100)],
      evidenceGap: "결측".repeat(100),
    })),
  }));
  const employeeEvidence = Object.fromEntries(Array.from({ length: 8 }, (_item, agentIndex) => [
    `agent-${agentIndex}`,
    Array.from({ length: 12 }, (_evidence, evidenceIndex) => evidence(`agent-${agentIndex}`, `official-${evidenceIndex + (agentIndex % 3) * 10}`)),
  ]));
  const result = buildMeetingSynthesisPrompt({
    topic: "전체 포트폴리오 분석",
    departmentReports: reports,
    employeeEvidence,
    directorContext: { positions: [{ symbol: "000880", quantity: "58" }] },
    shadowBoundary: { mode: "SHADOW_ONLY", liveOrderAllowed: false },
    outputContract: "확인하지 못한 값은 만들지 않는다.",
  });
  assert.ok(Array.from(result.prompt).length <= 44_000);
  assert.equal(result.trace.evidenceOccurrenceCount, 96);
  assert.equal(result.trace.uniqueEvidenceCount, 32);
  assert.equal(result.trace.staged, true);
  assert.match(result.prompt, /allEvidenceIds/);
});

test("최종 종합 실패 시 완료된 부서 요약과 실패 이유를 잃지 않는다", () => {
  const synthesis = failedMeetingSynthesis([
    { departmentName: "리서치부", summary: "공시와 뉴스 근거를 검토했습니다." },
    { departmentName: "리스크관리부", summary: "손실 한도 검토가 필요합니다." },
  ], "응답 계약 검증 실패");
  assert.equal(synthesis.decision, "hold");
  assert.match(synthesis.summary, /리서치부/);
  assert.match(synthesis.summary, /응답 계약 검증 실패/);
  assert.equal(synthesis.backtestRecommendation.required, false);
});

test("최종 종합 실패 기록도 전체 부서 원문과 입력 추적치를 함께 저장한다", () => {
  const reports = {
    research: { departmentName: "리서치부", summary: "리서치 원문".repeat(1_000) },
    risk: { departmentName: "리스크관리부", summary: "리스크 원문".repeat(1_000) },
  };
  const synthesis = failedMeetingSynthesis(Object.values(reports), "timeout");
  const content = buildMeetingAnalysisContent({
    topic: "전체 포트폴리오 분석",
    reports,
    synthesis,
    synthesisTrace: {
      schemaVersion: "investa.meeting-synthesis-input.v1",
      staged: true,
      departmentCount: 2,
      evidenceOccurrenceCount: 93,
      uniqueEvidenceCount: 31,
      includedEvidenceCount: 31,
      omittedEvidenceCount: 0,
      promptCharacterCount: 31_585,
      promptLimit: 44_000,
    },
    synthesisError: "timeout",
    portfolioCharts: [],
    telegramEvidence: { includedCount: 5 },
    evidenceSources: [{ evidenceId: "telegram-1", cited: true }],
  });
  const serialized = JSON.stringify(content);
  assert.equal(content.type, "meeting");
  assert.equal(content.reports, reports);
  assert.match(serialized, /리서치 원문/);
  assert.equal(content.synthesisTrace?.uniqueEvidenceCount, 31);
  assert.equal(content.synthesisError, "timeout");
  assert.equal(content.evidenceSources[0].evidenceId, "telegram-1");
});
