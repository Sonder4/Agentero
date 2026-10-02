/**
 * Bulk (whole-document) translation of body-text layout regions.
 *
 * Its own hook because it shares nothing with hover or with the analysis run
 * beyond the region list it reads: one abortable job, progressive overlay items,
 * and the toolbar button's start → stop → retry/clear flow. When layout regions
 * are missing the job queues (`waitingLayout`) behind the layout-analysis run
 * and auto-starts once regions land.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useStore } from "zustand";
import { backgroundTasksStore } from "@/lib/core/background-tasks";
import { errorText } from "@/lib/core/error";
import { notifyAction, notifyError, notifySuccess } from "@/lib/core/notify";
import { sameRelPaperPath } from "@/lib/core/path";
import {
	applyLayoutTranslateSidecar,
	currentLayoutTranslateCacheKey,
	enqueuePaperLayoutAnalysis,
	glossaryContentHash,
	groupLayoutTranslateItemsByPage,
	hasPendingLayoutTranslateItems,
	isLayoutTranslateUnitStaleForGlossary,
	type LayoutTranslateGlossaryTerm,
	type LayoutTranslateItem,
	type LayoutTranslateJobStatus,
	type LayoutTranslateState,
	layoutAnalysisStore,
	layoutDocumentKey,
	listTranslatableLayoutRegions,
	normalizeLayoutPaperKey,
	type PdfLayoutRegion,
	persistLayoutTranslateSidecarBestEffort,
	readLayoutTranslateGlossary,
	readLayoutTranslateSidecar,
	readLayoutTranslateState,
	runLayoutRegionTranslate,
	toLayoutTranslateItems,
	usedGlossaryTermHashes,
	writeLayoutTranslateSidecar,
	writeLayoutTranslateState,
} from "@/lib/pdf/layout";
import { displayTranslateError } from "@/lib/translate";

function sourceContentHash(source: string): string {
	let hash = 0x811c9dc5;
	for (let index = 0; index < source.length; index += 1) {
		hash ^= source.charCodeAt(index);
		hash = Math.imul(hash, 0x01000193);
	}
	return (hash >>> 0).toString(36);
}

export type UsePdfLayoutTranslateOptions = {
	docId: string;
	/** Translation-only panes hydrate completed overlays from the shared cache. */
	translationPane?: boolean;
	/** Pre-merge regions from {@link usePdfLayoutRegions}; the translate source. */
	layoutRawRegions: PdfLayoutRegion[] | null;
	/** Paper folder path; when present, full-document translations cache under source/. */
	paperAbsPath?: string | null;
	/** Vault-relative paper path; matches background-task rows to this paper. */
	paperRelPath?: string | null;
	/** Stable paper identifier for per-document Agent session reuse. */
	paperKey?: string | null;
	/** Catalog metadata id; stable when the paper folder is moved. */
	paperObjectId?: string | null;
	/** Vault root passed to the Agent as its cwd. */
	vaultPath?: string | null;
};

export type PdfLayoutTranslate = {
	/** Progressive bulk-translate overlays (body text / abstract / header), bucketed by page. */
	layoutTranslateItemsByPage: ReadonlyMap<
		number,
		readonly LayoutTranslateItem[]
	>;
	/** One-page tag state, keyed by 0-based page index. */
	layoutTranslatePageStateByPage: ReadonlyMap<
		number,
		{
			active: boolean;
			running: boolean;
			pending: boolean;
			done: boolean;
			error: boolean;
			doneCount: number;
			totalCount: number;
		}
	>;
	layoutTranslateProgress: {
		total: number;
		pending: number;
		running: number;
		done: number;
		error: number;
		skipped: number;
	};
	layoutTranslateRunning: boolean;
	/** Queued behind layout analysis; toolbar shows the waiting state. */
	layoutTranslateWaiting: boolean;
	/** Running, or finished with overlays still painted. */
	layoutTranslateActive: boolean;
	/** Sidecar hydration has completed for the current paper. */
	layoutTranslateCacheReady: boolean;
	/** Toolbar button label for the current job phase. */
	layoutTranslateLabel: string;
	/** Toolbar button: start → stop → retry incomplete / clear complete. */
	toggleLayoutTranslate: () => void;
	/** Per-page tag: retry incomplete blocks, translate this page, or hide a complete page. */
	togglePageLayoutTranslate: (pageIndex: number) => void;
};

/** What a queued job should start once layout regions land. */
type LayoutTranslateWaitTarget =
	| { mode: "document" }
	| { mode: "page"; pageIndex: number };

function mergeTranslatePageItems(
	items: readonly LayoutTranslateItem[],
	pageIndex: number,
	pageItems: readonly LayoutTranslateItem[],
): LayoutTranslateItem[] {
	return [
		...items.filter((it) => it.pageIndex !== pageIndex),
		...pageItems,
	].sort(
		(a, b) =>
			a.pageIndex - b.pageIndex ||
			a.readingOrder - b.readingOrder ||
			a.bbox.y - b.bbox.y ||
			a.bbox.x - b.bbox.x,
	);
}

/** Keep successful blocks when retrying a partial job, even before cache debounce flushes. */
function reuseCompletedTranslateItems(
	pending: readonly LayoutTranslateItem[],
	current: readonly LayoutTranslateItem[],
): LayoutTranslateItem[] {
	const byId = new Map(current.map((item) => [item.id, item]));
	return pending.map((item) => {
		const previous = byId.get(item.id);
		if (
			previous?.source === item.source &&
			previous.status === "done" &&
			previous.translated?.trim()
		) {
			return {
				...item,
				status: "done" as const,
				translated: previous.translated.trim(),
				error: undefined,
			};
		}
		return { ...item };
	});
}

/** Release interrupted blocks immediately; a cancelled runner may settle much later. */
function resetRunningTranslateItems(
	items: readonly LayoutTranslateItem[],
): LayoutTranslateItem[] {
	return items.map((item) =>
		item.status === "running" ? { ...item, status: "pending" } : item,
	);
}

async function invalidateChangedGlossaryItems(
	items: readonly LayoutTranslateItem[],
	paperAbsPath: string | null | undefined,
	objectId: string | null | undefined,
	onGlossary?: (terms: readonly LayoutTranslateGlossaryTerm[]) => void,
): Promise<LayoutTranslateItem[]> {
	if (!paperAbsPath) return items.map((item) => ({ ...item }));
	const key = currentLayoutTranslateCacheKey();
	const [glossary, state] = await Promise.all([
		readLayoutTranslateGlossary(
			paperAbsPath,
			objectId ?? paperAbsPath,
			key.sourceLang,
			key.targetLang,
		),
		readLayoutTranslateState(paperAbsPath),
	]);
	onGlossary?.(glossary.terms);
	if (
		!state ||
		state.objectId !== (objectId ?? paperAbsPath) ||
		state.cacheKey.providerId !== key.providerId ||
		state.cacheKey.sourceLang !== key.sourceLang ||
		state.cacheKey.targetLang !== key.targetLang ||
		state.cacheKey.serviceKey !== key.serviceKey
	) {
		return items.map((item) => ({ ...item }));
	}
	return items.map((item) => {
		if (
			!isLayoutTranslateUnitStaleForGlossary(
				state.units[item.id],
				item.source,
				glossary.terms,
			)
		) {
			return { ...item };
		}
		return {
			...item,
			status: "pending" as const,
			translated: undefined,
			error: undefined,
		};
	});
}

export function usePdfLayoutTranslate({
	docId,
	layoutRawRegions,
	translationPane = false,
	paperAbsPath,
	paperRelPath,
	paperKey,
	paperObjectId,
	vaultPath,
}: UsePdfLayoutTranslateOptions): PdfLayoutTranslate {
	const { t } = useTranslation("viewer");
	void translationPane;
	/** Progressive layout bulk-translate overlays (body text / abstract / header). */
	const [layoutTranslateJob, setLayoutTranslateJob] = useState<{
		status: LayoutTranslateJobStatus;
		items: LayoutTranslateItem[];
	}>({ status: "idle", items: [] });
	const layoutTranslateJobRef = useRef(layoutTranslateJob);
	layoutTranslateJobRef.current = layoutTranslateJob;
	const layoutTranslateAbortRef = useRef<AbortController | null>(null);
	const hiddenPageIndexesRef = useRef(new Set<number>());
	const hiddenDocumentRef = useRef(false);
	const cacheHydrationPathRef = useRef<string | null>(null);
	const stateRunIdRef = useRef(`${docId}:${Date.now()}`);
	const stateWriteRef = useRef<Promise<void>>(Promise.resolve());
	const glossaryTermsRef = useRef<readonly LayoutTranslateGlossaryTerm[]>([]);
	const translationObjectId = paperObjectId ?? paperKey ?? paperAbsPath;
	const [layoutTranslateCacheReady, setLayoutTranslateCacheReady] = useState(
		!paperAbsPath,
	);
	const queueStateWrite = useCallback(
		(items: readonly LayoutTranslateItem[], runId = stateRunIdRef.current) => {
			if (!paperAbsPath) return stateWriteRef.current;
			const cacheKey = currentLayoutTranslateCacheKey();
			stateWriteRef.current = stateWriteRef.current
				.catch(() => undefined)
				.then(async () => {
					const glossary = await readLayoutTranslateGlossary(
						paperAbsPath,
						translationObjectId ?? "paper",
						cacheKey.sourceLang,
						cacheKey.targetLang,
					);
					glossaryTermsRef.current = glossary.terms;
					const now = new Date().toISOString();
					const state: LayoutTranslateState = {
						schemaVersion: 1,
						runId,
						objectType: "paper",
						objectId: translationObjectId ?? "paper",
						cacheKey: {
							providerId: cacheKey.providerId,
							sourceLang: cacheKey.sourceLang,
							targetLang: cacheKey.targetLang,
							serviceKey: cacheKey.serviceKey,
							glossaryHash: glossaryContentHash(glossary.terms),
						},
						units: Object.fromEntries(
							items.map((item) => [
								item.id,
								{
									sourceHash: sourceContentHash(item.source),
									regionIds: [item.id],
									status: item.status,
									attempts: item.status === "running" ? 1 : 0,
									usedTermHashes: usedGlossaryTermHashes(
										item.source,
										glossary.terms,
									),
									lastError: item.error ?? null,
									updatedAt: now,
								},
							]),
						),
						updatedAt: now,
					};
					await writeLayoutTranslateState(paperAbsPath, state);
				})
				.catch(() => undefined);
			return stateWriteRef.current;
		},
		[paperAbsPath, translationObjectId],
	);
	/** Bumps when a page is hidden or shown without changing stored items. */
	const [hiddenRevision, setHiddenRevision] = useState(0);
	/** Queued target to auto-start once layout regions land. */
	const layoutTranslateWaitTargetRef = useRef<LayoutTranslateWaitTarget | null>(
		null,
	);
	const waitingAutoStartedRef = useRef(false);
	const layoutTranslateWaitToastIdRef = useRef(
		`layout-translate-wait:${docId}`,
	);
	layoutTranslateWaitToastIdRef.current = `layout-translate-wait:${docId}`;

	// Is a layout analysis queued/running for this paper? Task rows are the only
	// signal covering host-job queueing and headless runs (synthetic docIds);
	// the store UI covers row-less in-viewer runs via the real documentId.
	const layoutJobPending = useStore(backgroundTasksStore, (s) =>
		s.tasks.some(
			(task) =>
				(task.kind === "layoutAnalyze" || task.kind === "layoutRun") &&
				(task.status === "queued" || task.status === "running") &&
				sameRelPaperPath(task.paperPath, paperRelPath),
		),
	);
	const layoutUiRunning = useStore(layoutAnalysisStore, (s) => {
		if (s.ui.stage !== "running") return false;
		if (s.activeDocumentId === docId) return true;
		const activePaper = s.activePaperAbsPath;
		return (
			activePaper != null &&
			paperAbsPath != null &&
			normalizeLayoutPaperKey(activePaper) ===
				normalizeLayoutPaperKey(paperAbsPath)
		);
	});
	const parsePendingForPaper = layoutJobPending || layoutUiRunning;

	const applyHiddenPages = useCallback(
		(items: readonly LayoutTranslateItem[]) => {
			const hidden = hiddenPageIndexesRef.current;
			if (hidden.size === 0) return items.map((it) => ({ ...it }));
			return items
				.filter((it) => !hidden.has(it.pageIndex))
				.map((it) => ({ ...it }));
		},
		[],
	);

	const stopLayoutTranslate = useCallback(() => {
		layoutTranslateAbortRef.current?.abort();
		layoutTranslateAbortRef.current = null;
		setLayoutTranslateJob((prev) =>
			prev.status === "running"
				? {
						...prev,
						status: "cancelled",
						items: resetRunningTranslateItems(prev.items),
					}
				: prev,
		);
	}, []);

	const clearLayoutTranslate = useCallback(() => {
		layoutTranslateAbortRef.current?.abort();
		layoutTranslateAbortRef.current = null;
		hiddenPageIndexesRef.current.clear();
		hiddenDocumentRef.current = true;
		setHiddenRevision((revision) => revision + 1);
	}, []);

	// `runLayoutRegionTranslate` settles per-chain errors into item state instead
	// of throwing, so a dead translation API would otherwise finish silently.
	const notifyTranslateItemErrors = useCallback(
		(items: readonly LayoutTranslateItem[]) => {
			const failed = items.find((it) => it.status === "error" && it.error);
			if (!failed) return;
			notifyError(t("pdf.layoutTranslate.failed"), {
				description: displayTranslateError(failed.error ?? ""),
			});
		},
		[t],
	);

	const cancelWaitingLayout = useCallback(() => {
		layoutTranslateWaitTargetRef.current = null;
		waitingAutoStartedRef.current = false;
		toast.dismiss(layoutTranslateWaitToastIdRef.current);
		setLayoutTranslateJob((prev) =>
			prev.status === "waitingLayout"
				? { status: "idle", items: prev.items }
				: prev,
		);
	}, []);

	/**
	 * Queue the job behind layout analysis: ensure a parse is pending, park the
	 * target, and let the auto-start effect fire once regions land. Items are
	 * kept so a later start reuses completed blocks.
	 */
	const enterWaitingLayout = useCallback(
		(target: LayoutTranslateWaitTarget) => {
			if (!parsePendingForPaper) {
				if (!paperAbsPath) {
					// Loose PDF with nothing to enqueue and nothing running.
					notifyError(t("pdf.layoutTranslate.needLayout"));
					return;
				}
				enqueuePaperLayoutAnalysis({
					paperAbsPath,
					onError: (e) => {
						if (layoutTranslateWaitTargetRef.current !== target) return;
						notifyError(t("pdf.layoutTranslate.parseFailedWhileWaiting"), {
							description: errorText(e),
						});
						cancelWaitingLayout();
					},
				});
			}
			layoutTranslateWaitTargetRef.current = target;
			waitingAutoStartedRef.current = false;
			setLayoutTranslateJob((prev) => ({
				status: "waitingLayout",
				items: prev.items,
			}));
			notifyAction(t("pdf.layoutTranslate.queuedToast"), {
				id: layoutTranslateWaitToastIdRef.current,
				actionLabel: t("pdf.layoutTranslate.cancelQueue"),
				onAction: cancelWaitingLayout,
				duration: 12000,
			});
		},
		[parsePendingForPaper, paperAbsPath, cancelWaitingLayout, t],
	);

	const startLayoutTranslate = useCallback(() => {
		const raw = layoutRawRegions;
		if (!raw?.length) {
			enterWaitingLayout({ mode: "document" });
			return;
		}
		const regions = listTranslatableLayoutRegions(raw);
		if (regions.length === 0) {
			notifyError(t("pdf.layoutTranslate.noText"));
			return;
		}
		layoutTranslateAbortRef.current?.abort();
		const ac = new AbortController();
		layoutTranslateAbortRef.current = ac;
		hiddenPageIndexesRef.current.clear();
		const cacheKey = currentLayoutTranslateCacheKey();
		const pendingItems = reuseCompletedTranslateItems(
			toLayoutTranslateItems(regions),
			layoutTranslateJobRef.current.items,
		);
		setLayoutTranslateJob({ status: "running", items: pendingItems });
		hiddenDocumentRef.current = false;
		void (async () => {
			const sidecar = await readLayoutTranslateSidecar(paperAbsPath, cacheKey);
			if (ac.signal.aborted) return;
			const items = await invalidateChangedGlossaryItems(
				applyLayoutTranslateSidecar(pendingItems, sidecar),
				paperAbsPath,
				translationObjectId,
				(terms) => {
					glossaryTermsRef.current = terms;
				},
			);
			if (ac.signal.aborted) return;
			const needsRun = hasPendingLayoutTranslateItems(items);
			setLayoutTranslateJob({
				status: needsRun ? "running" : "done",
				items,
			});
			if (!needsRun) return;
			const finalItems = await runLayoutRegionTranslate({
				items,
				signal: ac.signal,
				paperKey,
				vaultPath,
				paperAbsPath,
				paperObjectId: translationObjectId,
				contextRegions: layoutRawRegions,
				onUpdate: (next) => {
					if (ac.signal.aborted) return;
					persistLayoutTranslateSidecarBestEffort(paperAbsPath, cacheKey, next);
					void queueStateWrite(next);
					setLayoutTranslateJob((prev) => ({
						status: prev.status === "cancelled" ? "cancelled" : "running",
						items: applyHiddenPages(next),
					}));
				},
			});
			// Cancellation/replacement owns the visible state now. A late result
			// must not restore cleared overlays or overwrite a newer sidecar write.
			if (ac.signal.aborted || layoutTranslateAbortRef.current !== ac) return;
			await writeLayoutTranslateSidecar(paperAbsPath, cacheKey, finalItems);
			void queueStateWrite(finalItems);
			notifyTranslateItemErrors(finalItems);
			setLayoutTranslateJob({
				status: hasPendingLayoutTranslateItems(finalItems) ? "partial" : "done",
				items: applyHiddenPages(finalItems),
			});
		})()
			.catch((e) => {
				if (ac.signal.aborted) return;
				const message = displayTranslateError(errorText(e));
				notifyError(t("pdf.layoutTranslate.failed"), { description: message });
				setLayoutTranslateJob((prev) => ({
					status: "partial",
					items: prev.items,
				}));
			})
			.finally(() => {
				if (layoutTranslateAbortRef.current === ac) {
					layoutTranslateAbortRef.current = null;
				}
			});
	}, [
		layoutRawRegions,
		paperAbsPath,
		translationObjectId,
		vaultPath,
		applyHiddenPages,
		enterWaitingLayout,
		notifyTranslateItemErrors,
		paperKey,
		queueStateWrite,
		t,
	]);

	const startPageLayoutTranslate = useCallback(
		(pageIndex: number) => {
			const raw = layoutRawRegions;
			if (!raw?.length) {
				enterWaitingLayout({ mode: "page", pageIndex });
				return;
			}
			const regions = listTranslatableLayoutRegions(raw).filter(
				(region) => region.pageIndex === pageIndex,
			);
			if (regions.length === 0) {
				notifyError(t("pdf.layoutTranslate.noText"));
				return;
			}
			layoutTranslateAbortRef.current?.abort();
			const ac = new AbortController();
			layoutTranslateAbortRef.current = ac;
			hiddenPageIndexesRef.current.delete(pageIndex);
			const cacheKey = currentLayoutTranslateCacheKey();
			const pendingItems = reuseCompletedTranslateItems(
				toLayoutTranslateItems(regions),
				layoutTranslateJobRef.current.items.filter(
					(item) => item.pageIndex === pageIndex,
				),
			);
			setLayoutTranslateJob((prev) => ({
				status: "running",
				items: mergeTranslatePageItems(
					resetRunningTranslateItems(prev.items),
					pageIndex,
					pendingItems,
				),
			}));
			hiddenDocumentRef.current = false;
			void (async () => {
				const sidecar = await readLayoutTranslateSidecar(
					paperAbsPath,
					cacheKey,
				);
				if (ac.signal.aborted) return;
				const pageItems = await invalidateChangedGlossaryItems(
					applyLayoutTranslateSidecar(pendingItems, sidecar),
					paperAbsPath,
					translationObjectId,
					(terms) => {
						glossaryTermsRef.current = terms;
					},
				);
				if (ac.signal.aborted) return;
				const needsRun = hasPendingLayoutTranslateItems(pageItems);
				setLayoutTranslateJob((prev) => {
					const merged = mergeTranslatePageItems(
						prev.items,
						pageIndex,
						pageItems,
					);
					return {
						status: needsRun
							? "running"
							: hasPendingLayoutTranslateItems(merged)
								? "partial"
								: "done",
						items: merged,
					};
				});
				if (!needsRun) return;
				const finalPageItems = await runLayoutRegionTranslate({
					items: pageItems,
					signal: ac.signal,
					paperKey,
					vaultPath,
					paperAbsPath,
					paperObjectId: translationObjectId,
					contextRegions: layoutRawRegions,
					onUpdate: (next) => {
						if (ac.signal.aborted) return;
						persistLayoutTranslateSidecarBestEffort(
							paperAbsPath,
							cacheKey,
							next,
							{
								preserveExisting: true,
								replacePageIndexes: [pageIndex],
							},
						);
						void queueStateWrite(next);
						setLayoutTranslateJob((prev) => ({
							status: prev.status === "cancelled" ? "cancelled" : "running",
							items: applyHiddenPages(
								mergeTranslatePageItems(prev.items, pageIndex, next),
							),
						}));
					},
				});
				// Only the current live run may publish or persist its final result.
				if (ac.signal.aborted || layoutTranslateAbortRef.current !== ac) return;
				await writeLayoutTranslateSidecar(
					paperAbsPath,
					cacheKey,
					finalPageItems,
					{
						preserveExisting: true,
						replacePageIndexes: [pageIndex],
					},
				);
				void queueStateWrite(finalPageItems);
				notifyTranslateItemErrors(finalPageItems);
				setLayoutTranslateJob((prev) => {
					const merged = mergeTranslatePageItems(
						prev.items,
						pageIndex,
						finalPageItems,
					);
					return {
						status: hasPendingLayoutTranslateItems(merged) ? "partial" : "done",
						items: applyHiddenPages(merged),
					};
				});
			})()
				.catch((e) => {
					if (ac.signal.aborted) return;
					const message = displayTranslateError(errorText(e));
					notifyError(t("pdf.layoutTranslate.failed"), {
						description: message,
					});
					setLayoutTranslateJob((prev) => ({
						status: "partial",
						items: prev.items,
					}));
				})
				.finally(() => {
					if (layoutTranslateAbortRef.current === ac) {
						layoutTranslateAbortRef.current = null;
					}
				});
		},
		[
			layoutRawRegions,
			paperAbsPath,
			translationObjectId,
			vaultPath,
			applyHiddenPages,
			enterWaitingLayout,
			notifyTranslateItemErrors,
			paperKey,
			queueStateWrite,
			t,
		],
	);

	const togglePageLayoutTranslate = useCallback(
		(pageIndex: number) => {
			// Waiting re-click cancels the queue instead of re-queueing.
			if (layoutTranslateJobRef.current.status === "waitingLayout") {
				cancelWaitingLayout();
				return;
			}
			const pageItems = layoutTranslateJobRef.current.items.filter(
				(item) => item.pageIndex === pageIndex,
			);
			if (pageItems.some((item) => item.status === "running")) {
				stopLayoutTranslate();
				return;
			}
			if (pageItems.length > 0 && hasPendingLayoutTranslateItems(pageItems)) {
				startPageLayoutTranslate(pageIndex);
				return;
			}
			const pageActive = pageItems.some((item) => item.translated?.trim());
			if (hiddenPageIndexesRef.current.has(pageIndex)) {
				hiddenPageIndexesRef.current.delete(pageIndex);
				setHiddenRevision((revision) => revision + 1);
				return;
			}
			if (pageActive) {
				hiddenPageIndexesRef.current.add(pageIndex);
				setHiddenRevision((revision) => revision + 1);
				return;
			}
			startPageLayoutTranslate(pageIndex);
		},
		[startPageLayoutTranslate, stopLayoutTranslate, cancelWaitingLayout],
	);

	const toggleLayoutTranslate = useCallback(() => {
		if (layoutTranslateJob.status === "waitingLayout") {
			cancelWaitingLayout();
			return;
		}
		if (layoutTranslateJob.status === "running") {
			stopLayoutTranslate();
			return;
		}
		if (layoutTranslateJob.status === "partial") {
			startLayoutTranslate();
			return;
		}
		if (
			layoutTranslateJob.status === "done" ||
			layoutTranslateJob.status === "cancelled"
		) {
			// Complete/explicitly-cancelled overlays keep the original clear action.
			if (layoutTranslateJob.items.some((it) => it.translated)) {
				if (hiddenDocumentRef.current) {
					hiddenDocumentRef.current = false;
					setHiddenRevision((revision) => revision + 1);
					return;
				}
				clearLayoutTranslate();
				return;
			}
		}
		startLayoutTranslate();
	}, [
		layoutTranslateJob,
		startLayoutTranslate,
		stopLayoutTranslate,
		clearLayoutTranslate,
		cancelWaitingLayout,
	]);

	// Abort bulk translate when switching documents.
	useEffect(() => {
		if (!docId) return;
		layoutTranslateAbortRef.current?.abort();
		layoutTranslateAbortRef.current = null;
		hiddenPageIndexesRef.current.clear();
		hiddenDocumentRef.current = false;
		layoutTranslateWaitTargetRef.current = null;
		waitingAutoStartedRef.current = false;
		setLayoutTranslateJob({ status: "idle", items: [] });
		return () => {
			layoutTranslateAbortRef.current?.abort();
		};
	}, [docId]);

	// A queued job starts the moment layout regions land (parse finished —
	// possibly in another tab via the sidecar file watcher).
	useEffect(() => {
		if (layoutTranslateJob.status !== "waitingLayout") return;
		if (!layoutRawRegions?.length) return;
		if (waitingAutoStartedRef.current) return;
		waitingAutoStartedRef.current = true;
		notifySuccess(t("pdf.layoutTranslate.startedAfterLayout"), {
			id: layoutTranslateWaitToastIdRef.current,
		});
		const target = layoutTranslateWaitTargetRef.current;
		layoutTranslateWaitTargetRef.current = null;
		if (target?.mode === "page") startPageLayoutTranslate(target.pageIndex);
		else startLayoutTranslate();
	}, [
		layoutTranslateJob.status,
		layoutRawRegions,
		startLayoutTranslate,
		startPageLayoutTranslate,
		t,
	]);

	// Edge-triggered parse-failure watch while queued: only tasks observed
	// queued/running during the wait may cancel it — finished rows linger in the
	// panel store, so a level check would trip on a stale earlier failure.
	useEffect(() => {
		if (layoutTranslateJob.status !== "waitingLayout") return;
		let failed = false;
		const fail = (message?: string) => {
			if (failed) return;
			failed = true;
			cancelWaitingLayout();
			notifyError(t("pdf.layoutTranslate.parseFailedWhileWaiting"), {
				description: message,
			});
		};
		const tracked = new Set<string>();
		const scanTasks = () => {
			for (const task of backgroundTasksStore.getState().tasks) {
				if (task.kind !== "layoutAnalyze" && task.kind !== "layoutRun")
					continue;
				if (!sameRelPaperPath(task.paperPath, paperRelPath)) continue;
				if (task.status === "queued" || task.status === "running") {
					tracked.add(task.id);
					continue;
				}
				if (
					tracked.has(task.id) &&
					(task.status === "failed" || task.status === "cancelled")
				) {
					fail();
					return;
				}
			}
		};
		const unsubTasks = backgroundTasksStore.subscribe(scanTasks);
		const sourceKey = layoutDocumentKey(docId).replace(/::translation$/, "");
		const uiMatches = (s: ReturnType<typeof layoutAnalysisStore.getState>) =>
			(s.activeDocumentId != null &&
				(layoutDocumentKey(s.activeDocumentId) === layoutDocumentKey(docId) ||
					layoutDocumentKey(s.activeDocumentId) === sourceKey)) ||
			(s.activePaperAbsPath != null &&
				paperAbsPath != null &&
				normalizeLayoutPaperKey(s.activePaperAbsPath) ===
					normalizeLayoutPaperKey(paperAbsPath));
		const unsubUi = layoutAnalysisStore.subscribe((s, prev) => {
			// Headless completion clears activePaperAbsPath; use its last running
			// snapshot to attribute the failure, including a source pane's run.
			if (
				!uiMatches(s) &&
				!(
					prev.ui.stage === "running" &&
					uiMatches(prev) &&
					prev.activeDocumentId === s.activeDocumentId
				)
			)
				return;
			if (s.ui.stage === "error") fail(s.ui.message);
			else if (s.ui.stage === "cancelled") fail();
		});
		scanTasks();
		return () => {
			unsubTasks();
			unsubUi();
		};
	}, [
		layoutTranslateJob.status,
		docId,
		paperAbsPath,
		paperRelPath,
		cancelWaitingLayout,
		t,
	]);

	// A translation pane must show the source pane's completed cache immediately;
	// it should not depend on starting a second translation job or on the source
	// pane's local React state.
	useEffect(() => {
		if (!paperAbsPath) {
			cacheHydrationPathRef.current = null;
			setLayoutTranslateCacheReady(true);
			return;
		}
		if (cacheHydrationPathRef.current !== paperAbsPath) {
			cacheHydrationPathRef.current = paperAbsPath;
			setLayoutTranslateCacheReady(false);
		}
		let cancelled = false;
		void readLayoutTranslateSidecar(
			paperAbsPath,
			currentLayoutTranslateCacheKey(),
		)
			.then(async (sidecar) => {
				if (cancelled) return;
				if (!sidecar?.items.length) {
					setLayoutTranslateCacheReady(true);
					return;
				}
				const items = await invalidateChangedGlossaryItems(
					layoutRawRegions?.length
						? applyLayoutTranslateSidecar(
								toLayoutTranslateItems(
									listTranslatableLayoutRegions(layoutRawRegions),
								),
								sidecar,
							)
						: sidecar.items.map((item) => ({
								...item,
								status: "done" as const,
							})),
					paperAbsPath,
					translationObjectId,
					(terms) => {
						glossaryTermsRef.current = terms;
					},
				);
				if (cancelled) return;
				setLayoutTranslateCacheReady(true);
				setLayoutTranslateJob((prev) => {
					if (
						prev.status === "running" ||
						prev.status === "waitingLayout" ||
						prev.items.some((item) => item.translated?.trim())
					) {
						return prev;
					}
					return {
						status: hasPendingLayoutTranslateItems(items) ? "partial" : "done",
						items,
					};
				});
			})
			.catch(() => {
				if (!cancelled) setLayoutTranslateCacheReady(true);
			});
		return () => {
			cancelled = true;
		};
	}, [paperAbsPath, translationObjectId, layoutRawRegions]);

	// Bucket once per job update (not per page); unchanged buckets keep their
	// previous array identity so memoized page overlays bail out while another
	// page streams.
	const layoutTranslateByPageRef = useRef<
		ReadonlyMap<number, readonly LayoutTranslateItem[]>
	>(new Map());
	// biome-ignore lint/correctness/useExhaustiveDependencies: hiddenRevision invalidates ref-backed visibility.
	const layoutTranslateItemsByPage = useMemo(() => {
		const hidden = hiddenPageIndexesRef.current;
		const visible = hiddenDocumentRef.current
			? []
			: hidden.size === 0
				? layoutTranslateJob.items
				: layoutTranslateJob.items.filter(
						(item) => !hidden.has(item.pageIndex),
					);
		const grouped = groupLayoutTranslateItemsByPage(
			visible,
			layoutTranslateByPageRef.current,
		);
		layoutTranslateByPageRef.current = grouped;
		return grouped;
	}, [layoutTranslateJob.items, hiddenRevision]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: hiddenRevision invalidates ref-backed visibility.
	const layoutTranslatePageStateByPage = useMemo(() => {
		const hidden = hiddenPageIndexesRef.current;
		const states = new Map<
			number,
			{
				active: boolean;
				running: boolean;
				pending: boolean;
				done: boolean;
				error: boolean;
				doneCount: number;
				totalCount: number;
			}
		>();
		if (hiddenDocumentRef.current) return states;
		for (const item of layoutTranslateJob.items) {
			if (hidden.has(item.pageIndex)) continue;
			const state = states.get(item.pageIndex) ?? {
				active: false,
				running: false,
				pending: false,
				done: false,
				error: false,
				doneCount: 0,
				totalCount: 0,
			};
			state.totalCount += 1;
			if (item.status === "running") {
				state.active = true;
				state.running = true;
			}
			if (item.status === "pending" || item.status === "skipped")
				state.pending = true;
			if (item.status === "error") state.error = true;
			if (item.translated?.trim()) {
				state.active = true;
				state.done = true;
				state.doneCount += 1;
			}
			states.set(item.pageIndex, state);
		}
		return states;
	}, [layoutTranslateJob.items, hiddenRevision]);

	const layoutTranslateProgress = useMemo(() => {
		const progress = {
			total: layoutTranslateJob.items.length,
			pending: 0,
			running: 0,
			done: 0,
			error: 0,
			skipped: 0,
		};
		for (const item of layoutTranslateJob.items) {
			if (item.status === "done" && item.translated?.trim()) progress.done += 1;
			else if (item.status === "running") progress.running += 1;
			else if (item.status === "error") progress.error += 1;
			else if (item.status === "skipped") progress.skipped += 1;
			else progress.pending += 1;
		}
		return progress;
	}, [layoutTranslateJob.items]);

	const layoutTranslateRunning = layoutTranslateJob.status === "running";
	const layoutTranslateWaiting = layoutTranslateJob.status === "waitingLayout";
	const layoutTranslateActive =
		!hiddenDocumentRef.current &&
		(layoutTranslateRunning ||
			layoutTranslateJob.items.some((it) => it.translated));
	const layoutTranslateLabel = layoutTranslateWaiting
		? t("pdf.layoutTranslate.waiting")
		: layoutTranslateRunning
			? t("pdf.layoutTranslate.stop")
			: layoutTranslateJob.status === "partial"
				? t("pdf.layoutTranslate.start")
				: layoutTranslateActive
					? t("pdf.layoutTranslate.clear")
					: t("pdf.layoutTranslate.start");

	return {
		layoutTranslateItemsByPage,
		layoutTranslatePageStateByPage,
		layoutTranslateProgress,
		layoutTranslateRunning,
		layoutTranslateWaiting,
		layoutTranslateActive,
		layoutTranslateCacheReady,
		layoutTranslateLabel,
		toggleLayoutTranslate,
		togglePageLayoutTranslate,
	};
}
