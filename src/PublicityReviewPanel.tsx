import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type ArticleStatus = "draft" | "rejected" | "approved" | "private_saved";
type ArticleRevision = {
  articleId: string; revision: number; title: string; bodyMarkdown: string; mediaIds: string[];
  links: string[]; maskingConfirmed: boolean; status: ArticleStatus; reviewNote?: string | null; createdAtMs: number;
};
type EvidencePack = { packId: string; artifacts: unknown[]; excludedEvidenceIds: string[]; confirmationRequired: string[] };
type MediaReview = { approvedMediaIds: string[]; rejected: string[] };

const ARTICLE_ID = "investa-development-log";
const splitValues = (value: string) => value.split(/[\n,]/).map((item) => item.trim()).filter(Boolean);

export function PublicityReviewPanel({ onMessage, onError }: { onMessage: (message: string) => void; onError: (message: string | null) => void }) {
  const [title, setTitle] = useState("Investa 개발 기록");
  const [body, setBody] = useState("검증된 코드와 테스트 근거만 정리합니다.");
  const [mediaText, setMediaText] = useState("");
  const [linksText, setLinksText] = useState("");
  const [maskingConfirmed, setMaskingConfirmed] = useState(false);
  const [reviewNote, setReviewNote] = useState("");
  const [saved, setSaved] = useState<ArticleRevision | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void invoke<ArticleRevision | null>("publicity_article_latest", { articleId: ARTICLE_ID }).then((article) => {
      if (!article) return;
      setSaved(article); setTitle(article.title); setBody(article.bodyMarkdown);
      setMediaText(article.mediaIds.join("\n")); setLinksText(article.links.join("\n"));
      setMaskingConfirmed(article.maskingConfirmed); setReviewNote(article.reviewNote ?? "");
    }).catch((reason) => onError(String(reason)));
  }, [onError]);

  const mediaIds = useMemo(() => splitValues(mediaText), [mediaText]);
  const links = useMemo(() => splitValues(linksText), [linksText]);
  const dirty = !saved || saved.title !== title.trim() || saved.bodyMarkdown !== body.trim()
    || saved.maskingConfirmed !== maskingConfirmed || saved.mediaIds.join("\n") !== mediaIds.join("\n")
    || saved.links.join("\n") !== links.join("\n");

  const save = async () => {
    setBusy(true);
    try {
      const article = await invoke<ArticleRevision>("publicity_article_draft_save", { request: { articleId: ARTICLE_ID, title, bodyMarkdown: body, mediaIds, links, maskingConfirmed } });
      setSaved(article); onMessage(`원고 revision ${article.revision}을 비공개 초안으로 저장했습니다.`); onError(null);
    } catch (reason) { onError(String(reason)); } finally { setBusy(false); }
  };
  const review = async (approved: boolean) => {
    if (!saved || dirty) { onError("수정 내용을 새 리비전으로 저장한 뒤 검토하세요."); return; }
    setBusy(true);
    try {
      const article = await invoke<ArticleRevision>(approved ? "publicity_article_approve" : "publicity_article_reject", { request: { articleId: saved.articleId, revision: saved.revision, note: reviewNote } });
      setSaved(article); onMessage(`원고 revision ${article.revision}을 ${approved ? "승인" : "반려"}했습니다.`); onError(null);
    } catch (reason) { onError(String(reason)); } finally { setBusy(false); }
  };
  const exportPackage = async () => {
    if (!saved || dirty || saved.status !== "approved") { onError("현재 저장 리비전의 대표 승인이 필요합니다."); return; }
    setBusy(true);
    try {
      const now = Date.now();
      const evidencePack = await invoke<EvidencePack>("publicity_evidence_pack_preview", { packId: `investa-${now}`, artifacts: [{ evidenceId: `build-${now}`, title: "로컬 검증 결과", sourceKind: "test", sourceReference: "cargo-test-and-pnpm-build", observedCompletion: "unit_tested", claimedCompletion: "unit_tested", verifiedAtMs: now, containsPersonalData: false, containsPrivateStrategy: false, license: "project-owned" }] });
      const mediaReview = await invoke<MediaReview>("publicity_media_review", { candidates: [] });
      const receipt = await invoke<{ directoryName: string; externalPublishPerformed: boolean }>("publicity_manual_package_export", { request: { packageId: `investa-${now}`, articleId: saved.articleId, revision: saved.revision, evidencePack, mediaReview } });
      onMessage(`수동 게시 패키지 ${receipt.directoryName} 생성 · 외부 게시 미수행`); onError(null);
    } catch (reason) { onError(String(reason)); } finally { setBusy(false); }
  };

  return <article className="readiness-publicity"><h4>홍보부 원고 검토·승인</h4>
    <p>제목·원고·미디어 ID·링크·마스킹을 한 리비전으로 저장합니다. 수정하면 기존 승인은 자동 무효화됩니다.</p>
    <label>제목<input value={title} onChange={(event) => setTitle(event.currentTarget.value)} /></label>
    <label>원고<textarea rows={6} value={body} onChange={(event) => setBody(event.currentTarget.value)} /></label>
    <label>미디어 ID<textarea rows={2} value={mediaText} placeholder="한 줄에 하나" onChange={(event) => setMediaText(event.currentTarget.value)} /></label>
    <label>공개 링크<textarea rows={2} value={linksText} placeholder="https:// 주소만 허용" onChange={(event) => setLinksText(event.currentTarget.value)} /></label>
    <label><input type="checkbox" checked={maskingConfirmed} onChange={(event) => setMaskingConfirmed(event.currentTarget.checked)} /> 개인정보·계좌·비밀정보 마스킹 확인</label>
    <label>검토 의견<input value={reviewNote} onChange={(event) => setReviewNote(event.currentTarget.value)} placeholder="수정·반려·승인 근거" /></label>
    <div className="readiness-actions">
      <button type="button" disabled={busy || !dirty || !title.trim() || !body.trim()} onClick={() => void save()}>새 리비전 저장</button>
      <button type="button" disabled={busy || dirty || !saved || reviewNote.trim().length < 2} onClick={() => void review(false)}>반려</button>
      <button type="button" disabled={busy || dirty || !saved || !maskingConfirmed || reviewNote.trim().length < 2} onClick={() => void review(true)}>대표 승인</button>
    </div>
    <p role="status">{saved ? `revision ${saved.revision} · ${dirty ? "저장되지 않은 수정" : saved.status}` : "저장된 원고 없음"}</p>
    <button type="button" disabled={busy || dirty || saved?.status !== "approved"} onClick={() => void exportPackage()}>승인 리비전 수동 게시 패키지 만들기</button>
  </article>;
}
