/**
 * Bulk page translation for layout body-text regions (text / abstract / header).
 * Progressive: callers apply each result as soon as it completes.
 */

import i18n from "@/i18n";
import {
	cancelAgentRun,
	listenAgentCompleted,
	listenAgentFailed,
	runOnce,
} from "@/lib/agent";
import { errorText } from "@/lib/core/error";
import { logger } from "@/lib/core/logger";
import { LAYOUT_SIDEBAR_MIN_SCORE } from "@/lib/pdf/layout/constants";
import {
	isAlgorithmLayoutKind,
	isLayoutTranslatableKind,
} from "@/lib/pdf/layout/labels";
import {
	LAYOUT_TRANSLATE_DATA_DIR,
	type LayoutTranslateGlossaryTerm,
	readLayoutTranslateGlossary,
	writeLayoutTranslateGlossary,
} from "@/lib/pdf/layout/layout-translate-object";
import {
	buildLayoutTranslateChains,
	type LayoutTranslateChain,
	normalizeLayoutSourceText,
	splitChainTranslation,
} from "@/lib/pdf/layout/layout-translate-source";
import { bboxCoveredBy } from "@/lib/pdf/layout/merge-captions";
import type {
	LayoutTranslateItem,
	LayoutTranslateItemStatus,
	LayoutTranslateRegion,
	PdfLayoutRegion,
} from "@/lib/pdf/layout/types";
import {
	evictAgentTranslateSessionId,
	getAgentTranslateSessionId,
	setAgentTranslateSessionId,
} from "@/lib/pdf/translate/agent-session-cache";
import { loadSettings } from "@/lib/settings";
import { runTranslate } from "@/lib/translate";
import {
	clampLayoutTranslateConcurrency,
	DEFAULT_LAYOUT_TRANSLATE_CONCURRENCY,
} from "@/lib/translate/defaults";
import { langsFromSettings } from "@/lib/translate/lang";
import {
	type MaskedToken,
	maskInlineTokens,
	restoreInlineTokens,
} from "@/lib/translate/mask";
import { resolveConfiguredTranslateAgent } from "@/lib/translate/resolve-agent";
import type {
	CommercialTranslateProviderId,
	TranslateProviderId,
	TranslateRunOptions,
	TranslateSettings,
} from "@/lib/translate/types";
import { joinVaultPath, readVaultFile, writeVaultFile } from "@/lib/vault";

const AGENT_TRANSLATION_TIMEOUT_MS = 180_000;
const AGENT_RETRY_LOG_RE =
	/Retrying\s*\(attempt\s*\d+\s*\/\s*\d+\s*,\s*waiting\s*\d+s\)\s*\.\.\.|Retry\s+finished,\s*resuming\.?/gi;

/** Remove ACP/provider retry chatter accidentally surfaced as assistant text. */
export function sanitizeAgentTranslationText(value: string): string {
	return value
		.replace(AGENT_RETRY_LOG_RE, "")
		.replace(/(?:^|\n)\s*(?:retrying|retry finished|resuming)[^\n]*/gi, "")
		.replace(/[ \t]{2,}/g, " ")
		.replace(/\n{3,}/g, "\n\n")
		.trim();
}

/** Soft cap per block to keep free-MT requests reasonable. */
export const LAYOUT_TRANSLATE_MAX_CHARS = 2500;

/** Parallel free/commercial MT workers (order of *start* follows reading order). */
export const LAYOUT_TRANSLATE_CONCURRENCY = 2;

/**
 * Soft cap on the summed payload per batch request. The Host rejects text over
 * 5000 chars (`MAX_TEXT_CHARS`), so keep batches comfortably below it while
 * still grouping ~one double-column page of paragraphs for shared context.
 */
export const LAYOUT_TRANSLATE_BATCH_CHARS = 4500;

export const LAYOUT_TRANSLATE_SIDECAR_SCHEMA_VERSION = 1;
export const LAYOUT_TRANSLATE_SIDECAR_FILE = "layout-translate.json";

/**
 * Trailing debounce for the whole-file `layout-translate.json` write. Each
 * translated block used to rewrite the entire sidecar immediately (400+ writes
 * for a long paper); coalescing keeps crash-recovery progress while bounding
 * disk churn. See paper-pipeline-orchestration.md §8.1.
 */
export const LAYOUT_TRANSLATE_WRITE_DEBOUNCE_MS = 500;

/** Pending debounced sidecar writes, keyed by paper folder. */
const translateSidecarWriteTimers = new Map<
	string,
	ReturnType<typeof setTimeout>
>();
const translateSidecarWriteQueues = new Map<string, Promise<void>>();

function enqueueTranslateSidecarWrite(
	paperAbsPath: string,
	write: () => Promise<void>,
): Promise<void> {
	const previous = translateSidecarWriteQueues.get(paperAbsPath);
	if (!previous) {
		let next: Promise<void>;
		try {
			next = write();
		} catch (error) {
			next = Promise.reject(error);
		}
		next = next.finally(() => {
			if (translateSidecarWriteQueues.get(paperAbsPath) === next) {
				translateSidecarWriteQueues.delete(paperAbsPath);
			}
		});
		translateSidecarWriteQueues.set(paperAbsPath, next);
		return next;
	}
	let next: Promise<void>;
	next = previous
		.catch(() => undefined)
		.then(write)
		.finally(() => {
			if (translateSidecarWriteQueues.get(paperAbsPath) === next) {
				translateSidecarWriteQueues.delete(paperAbsPath);
			}
		});
	translateSidecarWriteQueues.set(paperAbsPath, next);
	return next;
}

export type {
	LayoutTranslateItem,
	LayoutTranslateItemStatus,
	LayoutTranslateRegion,
} from "@/lib/pdf/layout/types";

export type LayoutTranslateJobStatus =
	| "idle"
	| "running"
	| "done"
	| "cancelled";

export type LayoutTranslateCacheKey = {
	providerId: TranslateProviderId;
	sourceLang: string;
	targetLang: string;
	serviceKey: string;
};

export type LayoutTranslateSidecarItem = {
	id: string;
	pageIndex: number;
	bbox: PdfLayoutRegion["bbox"];
	kind: PdfLayoutRegion["kind"];
	readingOrder: number;
	source: string;
	translated: string;
};

export type LayoutTranslateSidecar = {
	schemaVersion: number;
	source: {
		mode: "pdf-layout-translate";
		generatedAt: string;
		providerId: TranslateProviderId;
		sourceLang: string;
		targetLang: string;
		serviceKey: string;
	};
	items: LayoutTranslateSidecarItem[];
};

export type LayoutTranslateWriteOptions = {
	/**
	 * Single-page translation writes only a subset of layout blocks. Preserve
	 * cached blocks from other pages instead of replacing the whole sidecar.
	 */
	preserveExisting?: boolean;
	/** Existing cached blocks on these pages are replaced by `items`. */
	replacePageIndexes?: readonly number[];
};

/** Prefer body extract; fall back to caption title for headers. */
export function layoutRegionSourceText(region: PdfLayoutRegion): string {
	return (region.text ?? region.title ?? "").replace(/\s+/g, " ").trim();
}

/** True when most of `region` sits inside an algorithm detection box. */
export function isInsideAlgorithmRegion(
	region: PdfLayoutRegion,
	algorithms: readonly PdfLayoutRegion[],
	coverage = 0.45,
): boolean {
	for (const alg of algorithms) {
		if (alg.pageIndex !== region.pageIndex) continue;
		if (bboxCoveredBy(region.bbox, alg.bbox) >= coverage) return true;
	}
	return false;
}

/** "Algorithm 1" / "Alg. 2" style titles — keep original, do not translate. */
export function isAlgorithmTitleText(text: string): boolean {
	const t = text.trim();
	if (!t) return false;
	return /^(algorithm|alg\.?)\s*\d/i.test(t);
}

/**
 * PP-DocLayoutV3 reference labels (mapped to kind `text` in LABEL_TO_KIND).
 * Raw `label` is still preserved on the region.
 */
export function isReferenceLayoutLabel(label: string): boolean {
	const k = label.trim().toLowerCase();
	return k === "reference" || k === "reference_content";
}

/** PP-DocLayoutV3 side-margin text (`aside_text` → kind text; keep raw label). */
export function isAsideTextLayoutLabel(label: string): boolean {
	return label.trim().toLowerCase() === "aside_text";
}

/** Section headings like "References" / "Bibliography" / "参考文献". */
export function isReferenceSectionTitle(text: string): boolean {
	const t = text.trim();
	if (!t || t.length > 64) return false;
	return /^(references?|bibliography|works\s+cited|参考文[献獻])\b/i.test(t);
}

/**
 * Reading-order list of regions with extractable source text
 * (body, abstract, headers, figure/table captions).
 * Skips algorithm / reference / aside_text regions (and text inside them).
 */
export function listTranslatableLayoutRegions(
	regions: readonly PdfLayoutRegion[],
	minScore: number = LAYOUT_SIDEBAR_MIN_SCORE,
): LayoutTranslateRegion[] {
	const algorithms = regions.filter(
		(r) => isAlgorithmLayoutKind(r.kind) && r.score >= minScore,
	);
	// reference / reference_content are stored as kind=text; use raw label.
	const referenceBlocks = regions.filter(
		(r) => isReferenceLayoutLabel(r.label) && r.score >= minScore,
	);
	const out: LayoutTranslateRegion[] = [];
	for (const r of regions) {
		// Never translate algorithm detections themselves.
		if (isAlgorithmLayoutKind(r.kind)) continue;
		// Bibliography entries from the layout model.
		if (isReferenceLayoutLabel(r.label)) continue;
		// Side-margin / running column text (e.g. arXiv strip when labeled aside_text).
		if (isAsideTextLayoutLabel(r.label)) continue;
		if (!isLayoutTranslatableKind(r.kind)) continue;
		if (!(r.score >= minScore)) continue;
		if (!(r.bbox.w > 0 && r.bbox.h > 0)) continue;
		// Pseudocode / lines inside an algorithm bbox stay in the original language.
		if (isInsideAlgorithmRegion(r, algorithms)) continue;
		// Text/headers nested inside a reference block (e.g. multi-line cites).
		if (isInsideAlgorithmRegion(r, referenceBlocks)) continue;
		const full = normalizeLayoutSourceText(layoutRegionSourceText(r), r.kind);
		if (!full) continue;
		if (isAlgorithmTitleText(full)) continue;
		if (isReferenceSectionTitle(full)) continue;
		const source =
			full.length > LAYOUT_TRANSLATE_MAX_CHARS
				? `${full.slice(0, LAYOUT_TRANSLATE_MAX_CHARS)}…`
				: full;
		out.push({
			id: r.id,
			pageIndex: r.pageIndex,
			bbox: r.bbox,
			kind: r.kind,
			readingOrder: r.readingOrder,
			source,
		});
	}
	out.sort(
		(a, b) =>
			a.pageIndex - b.pageIndex ||
			a.readingOrder - b.readingOrder ||
			a.bbox.y - b.bbox.y ||
			a.bbox.x - b.bbox.x,
	);
	return out;
}

export function toLayoutTranslateItems(
	regions: readonly LayoutTranslateRegion[],
): LayoutTranslateItem[] {
	return regions.map((r) => ({ ...r, status: "pending" as const }));
}

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function isFiniteNumber(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value);
}

function parseBbox(value: unknown): PdfLayoutRegion["bbox"] | null {
	if (!isObject(value)) return null;
	const { x, y, w, h } = value;
	if (
		!isFiniteNumber(x) ||
		!isFiniteNumber(y) ||
		!isFiniteNumber(w) ||
		!isFiniteNumber(h)
	) {
		return null;
	}
	return { x, y, w, h };
}

/** FNV-1a 32-bit fingerprint of the custom translate prompt — cache-busting only. */
function promptFingerprint(prompt: string): string {
	let h = 0x811c9dc5;
	for (let i = 0; i < prompt.length; i++) {
		h ^= prompt.charCodeAt(i);
		h = Math.imul(h, 0x01000193);
	}
	return (h >>> 0).toString(36);
}

/**
 * Service identity for the sidecar cache. A non-empty custom prompt appends
 * its fingerprint so changing the prompt re-translates instead of hitting the
 * old cache; empty keeps the prompt-less key byte-identical so existing
 * `layout-translate.json` caches survive upgrades.
 */
export function translateServiceKey(settings: TranslateSettings): string {
	const providerId = settings.provider;
	const promptPart =
		settings.customPrompt.trim().length > 0
			? `:p${promptFingerprint(settings.customPrompt)}`
			: "";
	if (providerId === "agent") {
		return `agent:${settings.agentId || "default"}:${settings.modelId || "default"}${promptPart}`;
	}
	const configs = settings.providerConfigs as Partial<
		Record<
			CommercialTranslateProviderId,
			{ baseUrl?: string; region?: string; model?: string }
		>
	>;
	const config = configs[providerId as CommercialTranslateProviderId];
	if (!config) return `${providerId}${promptPart}`;
	return (
		[
			providerId,
			config.baseUrl?.trim() ?? "",
			config.region?.trim() ?? "",
			config.model?.trim() ?? "",
		].join(":") + promptPart
	);
}

export function currentLayoutTranslateCacheKey(): LayoutTranslateCacheKey {
	const settings = loadSettings();
	const langs = langsFromSettings(settings.translate, i18n.language ?? "en");
	return {
		providerId: settings.translate.provider,
		sourceLang: langs.sourceLang,
		targetLang: langs.targetLang,
		serviceKey: translateServiceKey(settings.translate),
	};
}

function sameLayoutTranslateCacheKey(
	a: LayoutTranslateCacheKey,
	b: LayoutTranslateCacheKey,
): boolean {
	return (
		a.providerId === b.providerId &&
		a.sourceLang === b.sourceLang &&
		a.targetLang === b.targetLang &&
		a.serviceKey === b.serviceKey
	);
}

export function layoutTranslateSidecarPath(paperAbsPath: string): string {
	// A loose PDF is treated as the paper unit named by its stem.  Imported
	// papers normally pass their unit directory directly (the directory that
	// contains metadata.json); this fallback keeps the path deterministic for
	// PDFs opened before they are imported into a unit directory.
	const paperUnit = paperAbsPath.replace(/\.pdf$/i, "");
	return joinVaultPath(
		joinVaultPath(paperUnit, LAYOUT_TRANSLATE_DATA_DIR),
		LAYOUT_TRANSLATE_SIDECAR_FILE,
	);
}

function parseLayoutTranslateSidecarItem(
	value: unknown,
): LayoutTranslateSidecarItem | null {
	if (!isObject(value)) return null;
	const { id, pageIndex, bbox, kind, readingOrder, source, translated } = value;
	if (
		typeof id !== "string" ||
		!isFiniteNumber(pageIndex) ||
		typeof kind !== "string" ||
		!isFiniteNumber(readingOrder) ||
		typeof source !== "string" ||
		typeof translated !== "string" ||
		!translated.trim()
	) {
		return null;
	}
	const parsedBbox = parseBbox(bbox);
	if (!parsedBbox) return null;
	return {
		id,
		pageIndex,
		bbox: parsedBbox,
		kind: kind as PdfLayoutRegion["kind"],
		readingOrder,
		source,
		translated,
	};
}

export function parseLayoutTranslateSidecar(
	raw: unknown,
	expectedKey?: LayoutTranslateCacheKey,
): LayoutTranslateSidecar | null {
	if (!isObject(raw)) return null;
	if (raw.schemaVersion !== LAYOUT_TRANSLATE_SIDECAR_SCHEMA_VERSION)
		return null;
	if (!isObject(raw.source) || raw.source.mode !== "pdf-layout-translate") {
		return null;
	}
	const { generatedAt, providerId, sourceLang, targetLang, serviceKey } =
		raw.source;
	if (
		typeof generatedAt !== "string" ||
		typeof providerId !== "string" ||
		typeof sourceLang !== "string" ||
		typeof targetLang !== "string" ||
		typeof serviceKey !== "string"
	) {
		return null;
	}
	const key: LayoutTranslateCacheKey = {
		providerId: providerId as TranslateProviderId,
		sourceLang,
		targetLang,
		serviceKey,
	};
	if (expectedKey && !sameLayoutTranslateCacheKey(key, expectedKey))
		return null;
	if (!Array.isArray(raw.items)) return null;
	const items = raw.items.map(parseLayoutTranslateSidecarItem);
	if (items.some((item) => !item)) return null;
	return {
		schemaVersion: LAYOUT_TRANSLATE_SIDECAR_SCHEMA_VERSION,
		source: {
			mode: "pdf-layout-translate",
			generatedAt,
			providerId: key.providerId,
			sourceLang,
			targetLang,
			serviceKey,
		},
		items: items as LayoutTranslateSidecarItem[],
	};
}

export async function readLayoutTranslateSidecar(
	paperAbsPath: string | null | undefined,
	key: LayoutTranslateCacheKey,
): Promise<LayoutTranslateSidecar | null> {
	if (!paperAbsPath) return null;
	try {
		const text = await readVaultFile(layoutTranslateSidecarPath(paperAbsPath));
		return parseLayoutTranslateSidecar(JSON.parse(text), key);
	} catch {
		return null;
	}
}

export function applyLayoutTranslateSidecar(
	items: readonly LayoutTranslateItem[],
	sidecar: LayoutTranslateSidecar | null,
): LayoutTranslateItem[] {
	if (!sidecar?.items.length) return items.map((it) => ({ ...it }));
	const byId = new Map(sidecar.items.map((item) => [item.id, item]));
	return items.map((item) => {
		const cached = byId.get(item.id);
		if (!cached || cached.source !== item.source) return { ...item };
		return {
			...item,
			status: "done" as const,
			translated: cached.translated.trim(),
			error: undefined,
		};
	});
}

export async function writeLayoutTranslateSidecar(
	paperAbsPath: string | null | undefined,
	key: LayoutTranslateCacheKey,
	items: readonly LayoutTranslateItem[],
	options: LayoutTranslateWriteOptions = {},
): Promise<void> {
	if (!paperAbsPath) return;
	const pending = translateSidecarWriteTimers.get(paperAbsPath);
	if (pending) {
		clearTimeout(pending);
		translateSidecarWriteTimers.delete(paperAbsPath);
	}
	await enqueueTranslateSidecarWrite(paperAbsPath, async () => {
		const done = items
			.filter((item) => item.status === "done" && item.translated?.trim())
			.map(
				(item): LayoutTranslateSidecarItem => ({
					id: item.id,
					pageIndex: item.pageIndex,
					bbox: item.bbox,
					kind: item.kind,
					readingOrder: item.readingOrder,
					source: item.source,
					translated: item.translated?.trim() ?? "",
				}),
			);
		const merged = new Map<string, LayoutTranslateSidecarItem>();
		if (options.preserveExisting) {
			const existing = await readLayoutTranslateSidecar(paperAbsPath, key);
			const replacePageIndexes = new Set(options.replacePageIndexes ?? []);
			for (const item of existing?.items ?? []) {
				if (replacePageIndexes.has(item.pageIndex)) continue;
				merged.set(item.id, item);
			}
		}
		for (const item of done) merged.set(item.id, item);
		const sidecar: LayoutTranslateSidecar = {
			schemaVersion: LAYOUT_TRANSLATE_SIDECAR_SCHEMA_VERSION,
			source: {
				mode: "pdf-layout-translate",
				generatedAt: new Date().toISOString(),
				providerId: key.providerId,
				sourceLang: key.sourceLang,
				targetLang: key.targetLang,
				serviceKey: key.serviceKey,
			},
			items: [...merged.values()].sort(
				(a, b) =>
					a.pageIndex - b.pageIndex ||
					a.readingOrder - b.readingOrder ||
					a.bbox.y - b.bbox.y ||
					a.bbox.x - b.bbox.x,
			),
		};
		await writeVaultFile(
			layoutTranslateSidecarPath(paperAbsPath),
			`${JSON.stringify(sidecar, null, 2)}\n`,
		);
	});
}

export function hasPendingLayoutTranslateItems(
	items: readonly LayoutTranslateItem[],
): boolean {
	return items.some(
		(item) => item.status !== "done" || !item.translated?.trim(),
	);
}

/** Chains translate as one unit, so one stale fragment re-runs all of them. */
function chainNeedsTranslate(chain: LayoutTranslateChain): boolean {
	return hasPendingLayoutTranslateItems(chain.members);
}

export function persistLayoutTranslateSidecarBestEffort(
	paperAbsPath: string | null | undefined,
	key: LayoutTranslateCacheKey,
	items: readonly LayoutTranslateItem[],
	options: LayoutTranslateWriteOptions = {},
): void {
	if (!paperAbsPath) return;
	const pending = translateSidecarWriteTimers.get(paperAbsPath);
	if (pending) clearTimeout(pending);
	const timer = setTimeout(() => {
		translateSidecarWriteTimers.delete(paperAbsPath);
		void writeLayoutTranslateSidecar(paperAbsPath, key, items, options).catch(
			(error) => {
				logger.warn("layout translate cache write failed", {
					error: errorText(error),
				});
			},
		);
	}, LAYOUT_TRANSLATE_WRITE_DEBOUNCE_MS);
	translateSidecarWriteTimers.set(paperAbsPath, timer);
}

/** Paint-relevant identity of one bucket slot (id, progress, partial text). */
function sameLayoutTranslateBucketSlot(
	before: LayoutTranslateItem | undefined,
	after: LayoutTranslateItem,
): boolean {
	return (
		before !== undefined &&
		before.id === after.id &&
		before.status === after.status &&
		before.translated === after.translated
	);
}

/**
 * Bucket job items by page so each page overlay reads its own list instead of
 * filtering the whole job. When `previous` is given, a bucket whose
 * paint-relevant contents are unchanged reuses the previous array identity, so
 * memoized page overlays bail out while the streaming job only touches the
 * page currently translating.
 */
export function groupLayoutTranslateItemsByPage(
	items: readonly LayoutTranslateItem[],
	previous?: ReadonlyMap<number, readonly LayoutTranslateItem[]>,
): ReadonlyMap<number, readonly LayoutTranslateItem[]> {
	const grouped = new Map<number, LayoutTranslateItem[]>();
	for (const item of items) {
		const bucket = grouped.get(item.pageIndex);
		if (bucket) bucket.push(item);
		else grouped.set(item.pageIndex, [item]);
	}
	if (!previous) return grouped;
	const next: Map<number, readonly LayoutTranslateItem[]> = new Map(grouped);
	for (const [pageIndex, bucket] of grouped) {
		const prev = previous.get(pageIndex);
		if (
			prev &&
			prev.length === bucket.length &&
			bucket.every((item, i) => sameLayoutTranslateBucketSlot(prev[i], item))
		) {
			next.set(pageIndex, prev);
		}
	}
	return next;
}

/** Non-streaming Agent runner for bulk layout translate (settings provider=agent). */
async function resolveLayoutTranslateAgentOpts(options: {
	paperKey: string | null | undefined;
	vaultPath: string | null | undefined;
	reuseSession: boolean;
}): Promise<TranslateRunOptions | undefined> {
	const settings = loadSettings();
	if (settings.translate.provider !== "agent") return undefined;
	const resolved = await resolveConfiguredTranslateAgent();
	if (!resolved.agentId) {
		throw new Error("No Agent configured for translation");
	}
	const agentId = resolved.agentId;
	const modelId = resolved.modelId;
	const { paperKey, vaultPath, reuseSession } = options;
	return {
		agent: {
			runOnce: async (prompt: string) => {
				const cachedSessionId = reuseSession
					? getAgentTranslateSessionId(paperKey, agentId, modelId)
					: undefined;
				const accepted = await runOnce({
					prompt,
					agentId,
					modelId,
					sessionId: cachedSessionId ?? undefined,
					vaultPath: vaultPath ?? undefined,
					workflow: "pdf-layout-translate",
					permissionMode: "auto",
					hideFromChatHistory: true,
				});
				const sessionId = accepted.sessionId;
				return await new Promise<string>((resolve, reject) => {
					let settled = false;
					let timeoutId: ReturnType<typeof setTimeout> | null = null;
					const unsubs: Array<() => void> = [];
					const cleanup = () => {
						if (timeoutId != null) clearTimeout(timeoutId);
						for (const unsubscribe of unsubs) unsubscribe();
					};
					const settle = (callback: () => void) => {
						if (settled) return;
						settled = true;
						cleanup();
						callback();
					};
					timeoutId = setTimeout(() => {
						void cancelAgentRun(sessionId).catch(() => undefined);
						if (reuseSession)
							evictAgentTranslateSessionId(paperKey, agentId, modelId);
						settle(() =>
							reject(
								new Error("Agent translation timed out after 180 seconds"),
							),
						);
					}, AGENT_TRANSLATION_TIMEOUT_MS);
					void listenAgentCompleted((ev) => {
						if (ev.sessionId !== sessionId) return;
						if (
							reuseSession &&
							ev.providerSessionId &&
							ev.stopReason !== "cancelled"
						) {
							setAgentTranslateSessionId(
								paperKey,
								agentId,
								modelId,
								ev.providerSessionId,
							);
						}
						const content = sanitizeAgentTranslationText(ev.content ?? "");
						settle(() =>
							content
								? resolve(content)
								: reject(new Error("Agent returned an empty translation")),
						);
					}).then((unsubscribe) => {
						if (settled) unsubscribe();
						else unsubs.push(unsubscribe);
					});
					void listenAgentFailed((ev) => {
						if (ev.sessionId !== sessionId) return;
						if (reuseSession)
							evictAgentTranslateSessionId(paperKey, agentId, modelId);
						settle(() =>
							reject(new Error(ev.error || "Agent translation failed")),
						);
					}).then((unsubscribe) => {
						if (settled) unsubscribe();
						else unsubs.push(unsubscribe);
					});
				});
			},
		},
	};
}

/**
 * Marker used to number paragraphs inside a batch payload, e.g. `[[1]] …`.
 * Double brackets distinguish it from single-bracket citations (`[1]`) that the
 * translation may legitimately contain.
 */
const TRANSLATE_BATCH_MARKER_RE = /(\[{2}|［{2})\s*(\d+)\s*(\]{2}|］{2})/g;

/** Anything batchable: a single region or a joined paragraph chain. */
type TranslateUnit = { source: string };

/** Projected payload length of a batch (matches {@link buildNumberedPayload}). */
function batchPayloadLength(batch: readonly TranslateUnit[]): number {
	if (batch.length === 0) return 0;
	if (batch.length === 1) return batch[0]?.source.length ?? 0;
	let total = 0;
	batch.forEach((unit, i) => {
		total += `[[${i + 1}]] `.length + unit.source.length;
		if (i > 0) total += 2; // "\n\n"
	});
	return total;
}

/**
 * Group units (already in reading order) into batches whose combined payload
 * stays under {@link LAYOUT_TRANSLATE_BATCH_CHARS}. A single unit always fits,
 * so nothing is dropped. Batches may span adjacent pages; results are mapped
 * back per unit, so positions are unaffected.
 */
export function buildTranslateBatches<T extends TranslateUnit>(
	units: readonly T[],
): T[][] {
	const batches: T[][] = [];
	let current: T[] = [];
	for (const unit of units) {
		const candidate = [...current, unit];
		if (
			current.length > 0 &&
			batchPayloadLength(candidate) > LAYOUT_TRANSLATE_BATCH_CHARS
		) {
			batches.push(current);
			current = [unit];
		} else {
			current = candidate;
		}
	}
	if (current.length > 0) batches.push(current);
	return batches;
}

/** Join a batch into one numbered payload; a lone unit is sent as-is. */
export function buildNumberedPayload(batch: readonly TranslateUnit[]): string {
	if (batch.length === 1) return batch[0]?.source ?? "";
	return batch.map((unit, i) => `[[${i + 1}]] ${unit.source}`).join("\n\n");
}

/**
 * Split a numbered translation back into `expected` segments by `[[n]]` markers.
 * Returns null when markers are missing/out of order or any segment is empty, so
 * the caller can fall back to translating each paragraph individually.
 */
export function parseNumberedTranslation(
	result: string,
	expected: number,
): string[] | null {
	const trimmed = result.trim();
	if (expected <= 1) return trimmed ? [trimmed] : null;
	const markers: { n: number; start: number; end: number }[] = [];
	const re = new RegExp(TRANSLATE_BATCH_MARKER_RE.source, "g");
	let m = re.exec(trimmed);
	while (m !== null) {
		markers.push({
			n: Number(m[2]),
			start: m.index,
			end: m.index + m[0].length,
		});
		m = re.exec(trimmed);
	}
	if (markers.length < expected) return null;
	const chosen = markers.slice(0, expected);
	for (let i = 0; i < expected; i++) {
		if (chosen[i]?.n !== i + 1) return null;
	}
	const segments: string[] = [];
	for (let i = 0; i < expected; i++) {
		const cur = chosen[i];
		if (!cur) return null;
		const nextStart = i + 1 < expected ? chosen[i + 1]?.start : trimmed.length;
		const seg = trimmed.slice(cur.end, nextStart ?? trimmed.length).trim();
		if (!seg) return null;
		segments.push(seg);
	}
	return segments;
}

/**
 * Translate regions with bounded concurrency. Invokes `onUpdate` after each
 * batch settles so the UI can paint overlays progressively.
 *
 * A paragraph continued in the next column or on the next page is one layout
 * region per fragment; those are chained into a single translation unit and the
 * result is split back per bbox. Units are then grouped into reading-order
 * batches translated in one numbered request so the engine sees surrounding
 * context; on a parse mismatch the batch falls back to per-unit translation.
 */
export async function runLayoutRegionTranslate(options: {
	items: LayoutTranslateItem[];
	signal?: AbortSignal;
	concurrency?: number;
	onUpdate: (items: LayoutTranslateItem[]) => void;
	paperKey?: string | null;
	paperObjectId?: string | null;
	vaultPath?: string | null;
	paperAbsPath?: string | null;
	contextRegions?: readonly PdfLayoutRegion[] | null;
}): Promise<LayoutTranslateItem[]> {
	const settings = loadSettings();
	const langs = langsFromSettings(settings.translate, i18n.language ?? "en");
	let glossary = await readLayoutTranslateGlossary(
		options.paperAbsPath,
		options.paperObjectId ??
			options.paperKey ??
			options.paperAbsPath ??
			"paper",
		langs.sourceLang,
		langs.targetLang,
	);
	const configuredConcurrency = clampLayoutTranslateConcurrency(
		Number(
			(
				settings.translate as TranslateSettings & {
					layoutTranslateConcurrency?: number;
				}
			).layoutTranslateConcurrency ??
				options.concurrency ??
				DEFAULT_LAYOUT_TRANSLATE_CONCURRENCY,
		),
	);
	const agentOpts = await resolveLayoutTranslateAgentOpts({
		paperKey: options.paperKey,
		vaultPath: options.vaultPath,
		reuseSession: configuredConcurrency === 1,
	});
	// Agent is heavy — serialize; free/commercial MT keeps a small pool.
	const concurrency = configuredConcurrency;
	const items = options.items.map((it) => ({ ...it }));
	const signal = options.signal;
	const allChains = buildLayoutTranslateChains(items);
	const pending = allChains.filter(chainNeedsTranslate);
	if (
		agentOpts?.agent &&
		glossary.terms.length === 0 &&
		options.paperAbsPath &&
		pending.length > 0
	) {
		const sample = allChains
			.filter(
				(_, index) =>
					index < 2 || index >= allChains.length - 2 || index % 10 === 0,
			)
			.slice(0, 8)
			.map((chain) => chain.source)
			.join("\n\n---\n\n");
		try {
			const raw = await agentOpts.agent.runOnce(
				`Extract a concise glossary for this research paper. Return JSON only in the form {"terms":[{"source":"...","aliases":[],"target":"...","category":"technical"}]}. Include only recurring technical terms, proper nouns, abbreviations, and established translations. Do not include formulas, URLs, numbers, or generic words. The target language is ${langs.targetLang}.\n\nPaper excerpts:\n${sample}`,
			);
			const jsonText = raw.match(/\{[\s\S]*\}/)?.[0];
			const parsed = jsonText
				? (JSON.parse(jsonText) as { terms?: unknown })
				: null;
			const terms = Array.isArray(parsed?.terms)
				? parsed.terms.filter((term): term is LayoutTranslateGlossaryTerm =>
						Boolean(
							term &&
								typeof term === "object" &&
								typeof (term as LayoutTranslateGlossaryTerm).source ===
									"string" &&
								typeof (term as LayoutTranslateGlossaryTerm).target ===
									"string",
						),
					)
				: [];
			if (terms.length > 0) {
				await writeLayoutTranslateGlossary(options.paperAbsPath, {
					schemaVersion: 1,
					objectType: "paper",
					objectId:
						options.paperObjectId ?? options.paperKey ?? options.paperAbsPath,
					sourceLang: langs.sourceLang,
					targetLang: langs.targetLang,
					terms,
				});
				glossary = { ...glossary, terms };
			}
		} catch (error) {
			logger.warn("layout translate glossary generation failed", {
				error: errorText(error),
			});
		}
	}
	const batches = buildTranslateBatches(pending);
	let nextBatch = 0;

	const publish = () => options.onUpdate(items.map((it) => ({ ...it })));

	const translateText = async (
		text: string,
		pageIndex: number | undefined,
		chainIndex?: number,
	): Promise<string> => {
		const current = chainIndex == null ? undefined : allChains[chainIndex];
		const previous = chainIndex == null ? undefined : allChains[chainIndex - 1];
		const next = chainIndex == null ? undefined : allChains[chainIndex + 1];
		const anchorPage = current?.members[0]?.pageIndex ?? pageIndex;
		const relatedRegions = (options.contextRegions ?? []).filter(
			(region) =>
				anchorPage == null ||
				(Math.abs(region.pageIndex - anchorPage) <= 1 &&
					Math.abs(
						region.readingOrder -
							(current?.members[0]?.readingOrder ?? region.readingOrder),
					) <= 12),
		);
		const relatedFormulas = relatedRegions
			.filter((region) => region.kind === "formula")
			.map((region) => region.text ?? region.title ?? "")
			.filter(Boolean)
			.slice(0, 6);
		const relatedTables = relatedRegions
			.filter((region) => region.kind === "table")
			.map((region) => region.text ?? region.title ?? "")
			.filter(Boolean)
			.slice(0, 3);
		const relatedFigures = relatedRegions
			.filter(
				(region) =>
					region.kind === "figure_title" ||
					region.kind === "image" ||
					region.kind === "chart",
			)
			.map((region) => region.text ?? region.title ?? "")
			.filter(Boolean)
			.slice(0, 3);
		const glossaryTerms: readonly LayoutTranslateGlossaryTerm[] = glossary.terms
			.filter((term) => {
				const source = current?.source ?? text;
				return [term.source, ...(term.aliases ?? [])].some((needle) =>
					source.toLocaleLowerCase().includes(needle.toLocaleLowerCase()),
				);
			})
			.slice(0, 24);
		const translated = await runTranslate(
			{
				text,
				context: {
					page: pageIndex != null ? pageIndex + 1 : undefined,
					surface: "pdf-layout-bulk",
					previousParagraph: previous?.source,
					nextParagraph: next?.source,
					previousTranslatedExcerpt: previous?.members
						.map((member) => member.translated?.trim())
						.filter(Boolean)
						.join(" ")
						.slice(-500),
					relatedFormulas,
					relatedTables,
					relatedFigures,
					glossary: glossaryTerms,
				},
			},
			agentOpts,
		);
		return sanitizeAgentTranslationText(translated.trim());
	};

	/** Restore masked tokens; retry unmasked when the engine ate a placeholder. */
	const finalizeSegment = async (
		chain: LayoutTranslateChain,
		segment: string,
		tokens: readonly MaskedToken[],
	): Promise<string> => {
		if (tokens.length === 0) return segment.trim();
		const restored = restoreInlineTokens(segment.trim(), tokens);
		if (restored.missing === 0) return restored.text;
		const retry = await translateText(
			chain.source,
			chain.members[0]?.pageIndex,
			allChains.indexOf(chain),
		);
		if (tokens.every((token) => retry.includes(token.original))) {
			return retry.trim();
		}
		const retryRestored = restoreInlineTokens(retry.trim(), tokens);
		if (retryRestored.missing !== 0) {
			throw new Error(
				`Protected token mismatch (${retryRestored.missing} token(s) missing)`,
			);
		}
		return retryRestored.text;
	};

	const applyChain = (chain: LayoutTranslateChain, translated: string) => {
		const segments = splitChainTranslation(
			translated,
			chain.members.map((m) => m.source.length),
		);
		chain.members.forEach((member, i) => {
			const segment = segments[i]?.trim();
			if (segment) {
				member.translated = segment;
				member.status = "done";
			} else {
				member.status = "error";
				member.error = "Empty translation result";
			}
		});
	};

	const settleError = (item: LayoutTranslateItem, e: unknown) => {
		if (signal?.aborted) {
			item.status = "skipped";
		} else {
			item.status = "error";
			item.error = errorText(e);
		}
	};

	const markChain = (
		chain: LayoutTranslateChain,
		status: LayoutTranslateItemStatus,
	) => {
		for (const member of chain.members) member.status = status;
	};

	const translateChain = async (chain: LayoutTranslateChain) => {
		const masked = maskInlineTokens(chain.source);
		const raw = await translateText(
			masked.text,
			chain.members[0]?.pageIndex,
			allChains.indexOf(chain),
		);
		const text = await finalizeSegment(chain, raw, masked.tokens);
		if (signal?.aborted) {
			markChain(chain, "skipped");
			return;
		}
		if (!text) {
			markChain(chain, "error");
			for (const member of chain.members) {
				member.error = "Empty translation result";
			}
			return;
		}
		applyChain(chain, text);
	};

	const worker = async () => {
		while (true) {
			if (signal?.aborted) return;
			const b = nextBatch;
			nextBatch += 1;
			if (b >= batches.length) return;
			const batch = batches[b];
			if (!batch || batch.length === 0) continue;
			for (const chain of batch) markChain(chain, "running");
			publish();
			try {
				if (signal?.aborted) {
					for (const chain of batch) markChain(chain, "skipped");
					publish();
					return;
				}
				const first = batch[0];
				if (batch.length === 1 && first) {
					await translateChain(first);
				} else {
					const maskedUnits = batch.map((chain) => {
						const masked = maskInlineTokens(chain.source);
						return { chain, source: masked.text, tokens: masked.tokens };
					});
					const result = await translateText(
						buildNumberedPayload(maskedUnits),
						first?.members[0]?.pageIndex,
						first ? allChains.indexOf(first) : undefined,
					);
					const segments = parseNumberedTranslation(result, batch.length);
					if (signal?.aborted) {
						for (const chain of batch) markChain(chain, "skipped");
					} else if (segments) {
						for (const [i, unit] of maskedUnits.entries()) {
							const text = await finalizeSegment(
								unit.chain,
								segments[i] ?? "",
								unit.tokens,
							);
							if (text) applyChain(unit.chain, text);
							else {
								markChain(unit.chain, "error");
								for (const member of unit.chain.members) {
									member.error = "Empty translation result";
								}
							}
						}
					} else {
						// Marker split failed — fall back to per-paragraph translation.
						for (const chain of batch) {
							if (signal?.aborted) {
								markChain(chain, "skipped");
								continue;
							}
							try {
								await translateChain(chain);
							} catch (e) {
								for (const member of chain.members) settleError(member, e);
							}
						}
					}
				}
			} catch (e) {
				for (const chain of batch) {
					for (const member of chain.members) settleError(member, e);
				}
			}
			publish();
		}
	};

	const pool = Array.from(
		{ length: Math.min(concurrency, Math.max(1, batches.length)) },
		() => worker(),
	);
	await Promise.all(pool);

	if (signal?.aborted) {
		for (const it of items) {
			if (it.status === "pending" || it.status === "running") {
				it.status = "skipped";
			}
		}
		publish();
	}

	return items;
}
