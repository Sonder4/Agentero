/**
 * Enqueue post-download / post-import layout analysis as a JobCenter task.
 * The renderer registers as the executor and runs local ONNX or the Paddle
 * API when Rust emits `job:offer` for a `layoutAnalyze` job.
 */

import i18n from "@/i18n";
import { commands } from "@/lib/core/bindings";
import { errorText } from "@/lib/core/error";
import { callApiResult } from "@/lib/core/ipc";
import { logger } from "@/lib/core/logger";
import { toVaultRelative } from "@/lib/core/path";
import {
	registerTaskExecutor,
	type TaskExecutorContext,
	type TaskReportArgs,
} from "@/lib/core/tasks";
import { analyzePaperLayoutHeadless } from "@/lib/pdf/layout/headless-analyze";
import { readLayoutSidecar } from "@/lib/pdf/layout/io";
import { layoutAnalysisStore } from "@/lib/pdf/layout/store";
import { joinVaultPath } from "@/lib/vault/path";
import { getVaultPath } from "@/lib/vault/store";

const queuedPapers = new Map<string, Array<(error: unknown) => void>>();

function normalizePaperKey(paperAbsPath: string): string {
	return paperAbsPath.replace(/[/\\]+$/, "").replace(/\\/g, "/");
}

/**
 * Register the renderer-side `layoutAnalyze` executor. Call before
 * `startTaskRuntime` (see `use-app-bootstrap`).
 */
export function registerLayoutTaskExecutor(): void {
	registerTaskExecutor("layoutAnalyze", runLayoutAnalyzeExecutor);
}

async function runLayoutAnalyzeExecutor(
	ctx: TaskExecutorContext,
): Promise<void> {
	const { jobId, vaultPath, paperPath, signal } = ctx;
	const paperAbsPath = paperPath
		? joinVaultPath(vaultPath, paperPath)
		: vaultPath;
	const paperLabel =
		paperPath?.split("/").filter(Boolean).pop() || paperAbsPath;
	const documentId = `headless-layout-${jobId}`;

	const report = (args: TaskReportArgs) =>
		ctx.report(args).catch((error) => {
			logger.warn("layout analyze job report failed", {
				jobId,
				error: errorText(error),
			});
		});

	const unsub = layoutAnalysisStore.subscribe((state) => {
		const { ui, activeDocumentId } = state;
		if (activeDocumentId !== documentId) return;
		if (ui.stage !== "running" || typeof ui.progress !== "number") return;
		void report({
			progress: ui.progress,
			phase: ui.message?.trim() || i18n.t("viewer:figures.analyzing"),
		});
	});

	try {
		if (signal.aborted) throw new Error("cancelled");
		await analyzePaperLayoutHeadless({
			paperAbsPath,
			paperLabel,
			documentId,
			signal,
		});
		await report({ progress: 100, phase: "completed", state: "succeeded" });
	} catch (e) {
		const message = errorText(e);
		const state = message.toLowerCase().includes("cancel")
			? "cancelled"
			: "failed";
		await report({ state, error: state === "failed" ? message : undefined });
	} finally {
		unsub();
	}
}

/**
 * After assets land on disk, ensure layout.json is produced. Enqueues a
 * JobCenter `layoutAnalyze` job; the `job:changed` projection owns the task
 * panel row, progress, and cancellation (§7.4 入口②). No-op when the sidecar
 * already exists or a job is already queued for this paper this session.
 */
export function enqueuePaperLayoutAnalysis(opts: {
	paperAbsPath: string;
	paperLabel?: string;
	onError?: (error: unknown) => void;
}): void {
	const paperAbsPath = opts.paperAbsPath;
	const paperKey = normalizePaperKey(paperAbsPath);
	if (!paperKey) return;
	const queuedErrors = queuedPapers.get(paperKey);
	if (queuedErrors) {
		if (opts.onError) queuedErrors.push(opts.onError);
		return;
	}

	const vaultPath = getVaultPath();
	if (!vaultPath) return;
	const paperRelPath = toVaultRelative(vaultPath, paperAbsPath);
	if (!paperRelPath) return;

	const onErrors = opts.onError ? [opts.onError] : [];
	queuedPapers.set(paperKey, onErrors);

	void (async () => {
		try {
			const cached = await readLayoutSidecar(paperAbsPath);
			if (cached?.regions?.length) return;
			await callApiResult(
				() =>
					commands.jobLayoutAnalyzeEnqueue({
						vaultPath,
						path: paperRelPath,
						lane: "normal",
						force: false,
					}),
				{ fallback: "layout analysis enqueue failed" },
			);
		} catch (e) {
			logger.warn("enqueue paper layout analysis failed", {
				paperAbsPath,
				error: errorText(e),
			});
			for (const onError of onErrors) onError(e);
		} finally {
			queuedPapers.delete(paperKey);
		}
	})();
}
