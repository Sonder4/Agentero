/**
 * Workspace action for opening a LaTeX-source translated PDF in a dual pane.
 */

import i18n from "@/i18n";
import {
	BackgroundTaskCancelledError,
	isBackgroundTaskCancelledError,
} from "@/lib/core/background-tasks";
import { errorText } from "@/lib/core/error";
import { logger } from "@/lib/core/logger";
import { notifyError } from "@/lib/core/notify";
import { runLocalActivity } from "@/lib/core/tasks";
import { isTauri } from "@/lib/core/tauri";
import { loadSettings } from "@/lib/settings";
import { langsFromSettings } from "@/lib/translate/lang";
import {
	createLatexTranslateRunner,
	latexTranslationPdfPath,
	prepareRootTexForLang,
	readLatexTranslationMeta,
	translatedTexPath,
	translateLatexProject,
	writeLatexTranslationMeta,
} from "@/lib/translate/latex-translate";
import type { FileNode } from "@/lib/vault/types";
import { dockHandle } from "@/lib/workspace/dock-registry";
import { getTabs, setTabs } from "@/lib/workspace/store";
import { createPlaceholderTab, tabIdForPath } from "@/lib/workspace/tabs/model";
import { translationSplitPlacement } from "@/lib/workspace/tabs/translation-split";
import type { DocTab } from "@/lib/workspace/tabs/types";
import { compileTexFile, resolveTexRoot } from "@/lib/workspace/tex-compile";

async function sha256FileText(text: string): Promise<string> {
	const encoder = new TextEncoder();
	const data = encoder.encode(text);
	const hashBuffer = await crypto.subtle.digest("SHA-256", data);
	const hashArray = Array.from(new Uint8Array(hashBuffer));
	return hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function needsReTranslation(
	paperPath: string,
	lang: string,
): Promise<boolean> {
	const meta = await readLatexTranslationMeta(paperPath, lang);
	if (!meta) return true;
	const settings = loadSettings();
	if (meta.providerId !== settings.translate.provider) return true;
	const { readVaultFile, vaultPathExists } = await import("@/lib/vault");
	for (const file of meta.files) {
		try {
			// If the user deleted translated files manually, re-translate.
			if (!(await vaultPathExists(file.translated))) return true;
			const current = await readVaultFile(file.original);
			const hash = await sha256FileText(current);
			if (hash !== file.sourceHash) return true;
		} catch {
			return true;
		}
	}
	return false;
}

function flattenFileNodes(nodes: FileNode[]): FileNode[] {
	const out: FileNode[] = [];
	for (const node of nodes) {
		out.push(node);
		if (node.children) out.push(...flattenFileNodes(node.children));
	}
	return out;
}

export async function hasLatexTranslationSource(
	paperPath: string,
): Promise<boolean> {
	return (await findRootTexPath(paperPath)) !== null;
}

async function findRootTexPath(paperPath: string): Promise<string | null> {
	if (!isTauri()) return null;
	// Probe the common arXiv layout.
	const candidates = ["main.tex", "ms.tex", "paper.tex"];
	const { readVaultFile, joinVaultPath, vaultPathExists, loadVaultTree } =
		await import("@/lib/vault");
	for (const candidate of candidates) {
		const p = joinVaultPath(joinVaultPath(paperPath, "source"), candidate);
		if (await vaultPathExists(p)) return p;
	}
	// Fall back to scanning source/ for a file containing \documentclass.
	const tree = await loadVaultTree(joinVaultPath(paperPath, "source"));
	for (const entry of flattenFileNodes(tree)) {
		if (!entry.path.endsWith(".tex")) continue;
		try {
			const content = await readVaultFile(entry.path);
			if (/\\documentclass/.test(content)) return entry.path;
		} catch {}
	}
	return null;
}

function throwIfCancelled(signal: AbortSignal): void {
	if (signal.aborted) throw new BackgroundTaskCancelledError();
}

/**
 * Translate the paper's LaTeX source to `{lang}`, compile it, and open the
 * resulting PDF in a right split pane. Runs as a local background activity so
 * the task panel shows progress and the toolbar button can reflect the running
 * state.
 */
export async function openLatexTranslationTab(
	paperTabId: string,
	paperPath: string,
	paperRelPath: string,
	paperId: string,
): Promise<void> {
	if (!isTauri()) {
		notifyError(i18n.t("viewer:pdf.latexTranslation.desktopOnly"));
		return;
	}

	const rootTexPath = await findRootTexPath(paperPath);
	if (!rootTexPath) {
		notifyError(i18n.t("viewer:pdf.latexTranslation.noSource"));
		return;
	}

	const settings = loadSettings();
	const langs = langsFromSettings(settings.translate, i18n.language ?? "en");
	const lang = langs.targetLang;

	try {
		await runLocalActivity(
			{
				kind: "latexTranslate",
				title: i18n.t("viewer:pdf.latexTranslation.tabTitle"),
				detail: i18n.t("viewer:pdf.latexTranslation.translating"),
				paperPath: paperRelPath,
			},
			async ({ signal, setProgress, setDetail }) => {
				throwIfCancelled(signal);

				if (await needsReTranslation(paperPath, lang)) {
					setProgress(0);
					const runner = createLatexTranslateRunner();
					const { rootTranslated, meta } = await translateLatexProject(
						rootTexPath,
						lang,
						runner,
						{
							onProgress: (translationPct) =>
								setProgress(Math.round((translationPct / 100) * 90)),
						},
					);
					throwIfCancelled(signal);
					await writeLatexTranslationMeta(paperPath, lang, meta);

					// Prepare the root tex for the target language (e.g. CJK support).
					const { readVaultFile, writeVaultFile } = await import("@/lib/vault");
					const rootContent = await readVaultFile(rootTranslated);
					const prepared = prepareRootTexForLang(rootContent, lang);
					await writeVaultFile(rootTranslated, prepared.text);
					setProgress(90);
				}

				const rootTranslated = translatedTexPath(rootTexPath, lang);
				const resolvedRoot = await resolveTexRoot(rootTranslated);
				const expectedPdf = latexTranslationPdfPath(paperPath, paperId, lang);

				setDetail(i18n.t("viewer:pdf.latexTranslation.compiling"));
				setProgress(90);
				const prepared = prepareRootTexForLang(
					await import("@/lib/vault").then((m) =>
						m.readVaultFile(resolvedRoot),
					),
					lang,
				);

				// Force xelatex for CJK languages.
				const engineOverride =
					prepared.engine === "xelatex" ? "xelatex" : undefined;

				const pdfPath = await compileTexFile(resolvedRoot, {
					quietSuccess: true,
					triggerPath: resolvedRoot,
					engine: engineOverride,
				});
				if (!pdfPath) {
					throw new Error(i18n.t("viewer:pdf.latexTranslation.compileFailed"));
				}

				throwIfCancelled(signal);

				// Move the compiled PDF next to the original paper PDF.
				const { renameVaultPath } = await import("@/lib/vault");
				await renameVaultPath(pdfPath, expectedPdf);
				setProgress(100);

				// Open or focus the right pane.
				openCompiledTranslationPane(paperTabId, paperPath, expectedPdf);
			},
		);
	} catch (e) {
		if (isBackgroundTaskCancelledError(e)) return;
		logger.error("latex translation failed", { error: errorText(e) });
		notifyError(errorText(e));
	}
}

function openCompiledTranslationPane(
	paperTabId: string,
	paperPath: string,
	pdfPath: string,
): void {
	const tabs = getTabs();
	const paperTab =
		tabs.find((t: DocTab) => t.id === paperTabId) ??
		tabs.find((t: DocTab) => t.id === tabIdForPath(paperPath));
	if (!paperTab) return;

	const translationId = `${tabIdForPath(paperPath)}::latex-translation`;
	const existing = tabs.find((t: DocTab) => t.id === translationId);
	if (existing) {
		dockHandle()?.activatePanel(existing.id);
		return;
	}

	const translationPane: DocTab = {
		...createPlaceholderTab(pdfPath, "pdf", translationId),
		kind: "paper",
		title: i18n.t("viewer:pdf.latexTranslation.tabTitle"),
		loaded: true,
	};

	setTabs((prev: DocTab[]) => [...prev, translationPane]);
	dockHandle()?.splitPanelRight(
		translationPane,
		translationSplitPlacement(paperTab.id, tabs).referencePanelId,
	);
}
