import { joinVaultPath, readVaultFile, writeVaultFile } from "@/lib/vault";

const stateWriteQueues = new Map<string, Promise<void>>();

function enqueueStateWrite(
	path: string,
	write: () => Promise<void>,
): Promise<void> {
	const previous = stateWriteQueues.get(path) ?? Promise.resolve();
	let next: Promise<void>;
	next = previous
		.catch(() => undefined)
		.then(write)
		.finally(() => {
			if (stateWriteQueues.get(path) === next) stateWriteQueues.delete(path);
		});
	stateWriteQueues.set(path, next);
	return next;
}

/** Internal paper data directory; `source/` remains reserved for raw layout data. */
export const LAYOUT_TRANSLATE_DATA_DIR = ".src";
export const LAYOUT_TRANSLATE_GLOSSARY_FILE = "glossary.json";
export const LAYOUT_TRANSLATE_STATE_FILE = "state.json";

export type LayoutTranslateGlossaryTerm = {
	source: string;
	aliases?: string[];
	target: string;
	category?: string;
	confidence?: string;
	frequency?: number;
	notes?: string;
};

export type LayoutTranslateGlossary = {
	schemaVersion: 1;
	objectType: "paper";
	objectId: string;
	sourceLang: string;
	targetLang: string;
	terms: LayoutTranslateGlossaryTerm[];
	contentHash: string;
	updatedAt: string;
};

export type LayoutTranslateUnitState = {
	sourceHash: string;
	regionIds: string[];
	status: "pending" | "running" | "done" | "error" | "skipped";
	attempts: number;
	usedTermHashes?: Record<string, string>;
	lastError?: string | null;
	updatedAt: string;
};

export type LayoutTranslateState = {
	schemaVersion: 1;
	runId: string;
	objectType: "paper";
	objectId: string;
	cacheKey: Record<string, string>;
	units: Record<string, LayoutTranslateUnitState>;
	updatedAt: string;
};

function objectSidecarPath(paperAbsPath: string, file: string): string {
	const paperUnit = paperAbsPath.replace(/\.pdf$/i, "");
	return joinVaultPath(
		joinVaultPath(paperUnit, LAYOUT_TRANSLATE_DATA_DIR),
		file,
	);
}

export function layoutTranslateGlossaryPath(paperAbsPath: string): string {
	return objectSidecarPath(paperAbsPath, LAYOUT_TRANSLATE_GLOSSARY_FILE);
}

export function layoutTranslateStatePath(paperAbsPath: string): string {
	return objectSidecarPath(paperAbsPath, LAYOUT_TRANSLATE_STATE_FILE);
}

function fnv1a(value: string): string {
	let hash = 0x811c9dc5;
	for (let i = 0; i < value.length; i++) {
		hash ^= value.charCodeAt(i);
		hash = Math.imul(hash, 0x01000193);
	}
	return (hash >>> 0).toString(36);
}

/** Stable compact hash for source snapshots stored in the resume state. */
export function sourceContentHash(source: string): string {
	return fnv1a(source);
}

export function glossaryContentHash(
	terms: readonly LayoutTranslateGlossaryTerm[],
): string {
	return fnv1a(JSON.stringify(terms));
}

/** Only terms present in this source can invalidate its completed translation. */
export function usedGlossaryTermHashes(
	source: string,
	terms: readonly LayoutTranslateGlossaryTerm[],
): Record<string, string> {
	const lower = source.toLocaleLowerCase();
	return Object.fromEntries(
		terms
			.filter((term) =>
				[term.source, ...(term.aliases ?? [])].some(
					(alias) =>
						alias.length > 0 && lower.includes(alias.toLocaleLowerCase()),
				),
			)
			.map((term) => [
				term.source.toLocaleLowerCase(),
				fnv1a(JSON.stringify(term)),
			]),
	);
}

export function isLayoutTranslateUnitStaleForGlossary(
	previous: LayoutTranslateUnitState | undefined,
	source: string,
	terms: readonly LayoutTranslateGlossaryTerm[],
): boolean {
	if (!previous || previous.sourceHash !== sourceContentHash(source))
		return false;
	const current = usedGlossaryTermHashes(source, terms);
	const used = previous.usedTermHashes ?? {};
	// Older state writers did not record term hashes. Preserve their completed
	// blocks until a run writes the richer per-unit metadata.
	if (Object.keys(used).length === 0) return false;
	const keys = new Set([...Object.keys(current), ...Object.keys(used)]);
	return [...keys].some((key) => current[key] !== used[key]);
}

export async function readLayoutTranslateGlossary(
	paperAbsPath: string | null | undefined,
	objectId: string,
	sourceLang: string,
	targetLang: string,
): Promise<LayoutTranslateGlossary> {
	const empty = (): LayoutTranslateGlossary => ({
		schemaVersion: 1,
		objectType: "paper",
		objectId,
		sourceLang,
		targetLang,
		terms: [],
		contentHash: glossaryContentHash([]),
		updatedAt: new Date().toISOString(),
	});
	if (!paperAbsPath) return empty();
	try {
		const parsed = JSON.parse(
			await readVaultFile(layoutTranslateGlossaryPath(paperAbsPath)),
		) as Partial<LayoutTranslateGlossary>;
		if (
			parsed.schemaVersion !== 1 ||
			parsed.objectType !== "paper" ||
			parsed.objectId !== objectId ||
			parsed.sourceLang !== sourceLang ||
			parsed.targetLang !== targetLang ||
			!Array.isArray(parsed.terms)
		) {
			return empty();
		}
		const terms = parsed.terms.filter(
			(term): term is LayoutTranslateGlossaryTerm =>
				Boolean(
					term &&
						typeof term.source === "string" &&
						typeof term.target === "string",
				),
		);
		return {
			...empty(),
			terms,
			contentHash: glossaryContentHash(terms),
			updatedAt:
				typeof parsed.updatedAt === "string"
					? parsed.updatedAt
					: new Date().toISOString(),
		};
	} catch {
		return empty();
	}
}

export async function writeLayoutTranslateGlossary(
	paperAbsPath: string,
	glossary: Omit<LayoutTranslateGlossary, "contentHash" | "updatedAt">,
): Promise<void> {
	const terms = glossary.terms.map((term) => ({ ...term }));
	const value: LayoutTranslateGlossary = {
		...glossary,
		terms,
		contentHash: glossaryContentHash(terms),
		updatedAt: new Date().toISOString(),
	};
	await writeVaultFile(
		layoutTranslateGlossaryPath(paperAbsPath),
		`${JSON.stringify(value, null, 2)}\n`,
	);
}

export async function readLayoutTranslateState(
	paperAbsPath: string | null | undefined,
): Promise<LayoutTranslateState | null> {
	if (!paperAbsPath) return null;
	try {
		const parsed = JSON.parse(
			await readVaultFile(layoutTranslateStatePath(paperAbsPath)),
		) as LayoutTranslateState;
		if (
			parsed.schemaVersion !== 1 ||
			parsed.objectType !== "paper" ||
			!parsed.units ||
			typeof parsed.units !== "object" ||
			!parsed.cacheKey ||
			typeof parsed.cacheKey !== "object"
		) {
			return null;
		}
		return parsed;
	} catch {
		return null;
	}
}

export async function writeLayoutTranslateState(
	paperAbsPath: string | null | undefined,
	state: LayoutTranslateState,
): Promise<void> {
	if (!paperAbsPath) return;
	const path = layoutTranslateStatePath(paperAbsPath);
	await enqueueStateWrite(path, async () => {
		const previous = await readLayoutTranslateState(paperAbsPath);
		const sameCacheKey =
			previous?.objectId === state.objectId &&
			["providerId", "sourceLang", "targetLang", "serviceKey"].every(
				(key) => previous.cacheKey[key] === state.cacheKey[key],
			);
		const units = sameCacheKey
			? { ...previous.units, ...state.units }
			: state.units;
		await writeVaultFile(
			path,
			`${JSON.stringify({ ...state, units, updatedAt: new Date().toISOString() }, null, 2)}\n`,
		);
	});
}
