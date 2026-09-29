/**
 * LaTeX-source translation for issue #591.
 *
 * Translates a local LaTeX project into `_{lang}.tex` siblings under `source/`,
 * rewrites \input/\include references, and produces a compiled PDF at
 * `{paper}/{paper_id}.{lang}.pdf`.
 */

import i18n from "@/i18n";
import { errorText } from "@/lib/core/error";
import { logger } from "@/lib/core/logger";
import { basenameOf, dirnameOf } from "@/lib/core/path";
import { loadSettings } from "@/lib/settings";
import { langsFromSettings } from "@/lib/translate/lang";
import { runTranslate } from "@/lib/translate/run";
import type { TranslateRunOptions } from "@/lib/translate/types";
import { joinVaultPath, readVaultFile, writeVaultFile } from "@/lib/vault";

const TRANSLATE_CHUNK_CHARS = 700;

export type LatexMaskedToken = {
	placeholder: string;
	original: string;
};

export type LatexMaskedText = {
	text: string;
	tokens: LatexMaskedToken[];
};

function placeholderFor(index: number): string {
	return `{{${index}}}`;
}

function isCommandChar(c: string): boolean {
	return /[A-Za-z]/.test(c);
}

function readBalanced(
	text: string,
	start: number,
	open: string,
	close: string,
): { value: string; end: number } | null {
	if (text[start] !== open) return null;
	let depth = 1;
	let i = start + 1;
	while (i < text.length && depth > 0) {
		if (text[i] === open) depth++;
		else if (text[i] === close) depth--;
		i++;
	}
	if (depth !== 0) return null;
	return { value: text.slice(start, i), end: i };
}

function readCommandWithArguments(text: string, start: number) {
	if (text[start] !== "\\") return null;
	let i = start + 1;
	while (i < text.length && isCommandChar(text[i])) i++;
	if (i === start + 1) {
		// Single non-letter escape such as \\, \%, \{.
		i++;
	}
	let end = i;

	// Read optional [] and mandatory {} arguments greedily in the order they
	// appear (e.g. \newtcolorbox{name}[2][]{spec}).
	while (end < text.length && (text[end] === "[" || text[end] === "{")) {
		if (text[end] === "[") {
			const bracket = readBalanced(text, end, "[", "]");
			if (!bracket) break;
			end = bracket.end;
		} else {
			const brace = readBalanced(text, end, "{", "}");
			if (!brace) break;
			end = brace.end;
		}
	}

	return { value: text.slice(start, end), end };
}

/**
 * Mask LaTeX commands (with optional [] and all consecutive nested {} arguments),
 * inline/display math, comments, and URLs/DOIs.
 */
export function maskLatexSource(text: string): LatexMaskedText {
	const tokens: LatexMaskedToken[] = [];
	let out = "";
	let i = 0;
	while (i < text.length) {
		// Skip existing placeholders so we do not double-wrap.
		const placeholderMatch = text.slice(i).match(/^\{\{\d+\}\}/);
		if (placeholderMatch) {
			out += placeholderMatch[0];
			i += placeholderMatch[0].length;
			continue;
		}

		// Comments.
		if (text[i] === "%") {
			const end = text.indexOf("\n", i);
			const comment = end === -1 ? text.slice(i) : text.slice(i, end);
			const ph = placeholderFor(tokens.length);
			tokens.push({ placeholder: ph, original: comment });
			out += ph;
			i += comment.length;
			continue;
		}

		// Display math $$...$$
		if (text.slice(i, i + 2) === "$$") {
			const end = text.indexOf("$$", i + 2);
			if (end !== -1) {
				const ph = placeholderFor(tokens.length);
				tokens.push({
					placeholder: ph,
					original: text.slice(i, end + 2),
				});
				out += ph;
				i = end + 2;
				continue;
			}
		}

		// Inline math $...$.
		if (text[i] === "$") {
			const end = text.indexOf("$", i + 1);
			if (end !== -1) {
				const ph = placeholderFor(tokens.length);
				tokens.push({
					placeholder: ph,
					original: text.slice(i, end + 1),
				});
				out += ph;
				i = end + 1;
				continue;
			}
		}

		// LaTeX command \name[...]{...}{...}...
		if (text[i] === "\\") {
			const cmd = readCommandWithArguments(text, i);
			if (cmd) {
				const ph = placeholderFor(tokens.length);
				tokens.push({ placeholder: ph, original: cmd.value });
				out += ph;
				i = cmd.end;
				continue;
			}
		}

		// URLs / DOIs.
		const urlMatch = text.slice(i).match(/^https?:\/\/[^\s、，。]+/);
		if (urlMatch) {
			const ph = placeholderFor(tokens.length);
			tokens.push({ placeholder: ph, original: urlMatch[0] });
			out += ph;
			i += urlMatch[0].length;
			continue;
		}

		out += text[i];
		i++;
	}
	return { text: out, tokens };
}

export function restoreLatexSource(
	text: string,
	tokens: readonly LatexMaskedToken[],
): { text: string; missing: number } {
	let out = text;
	let missing = 0;
	for (const token of tokens) {
		const body = token.placeholder.slice(2, -2);
		if (!new RegExp(`\\{\\{\\s*${body}\\s*\\}\\}`).test(out)) {
			missing += 1;
			continue;
		}
		out = out.replace(
			new RegExp(`\\{\\{\\s*${body}\\s*\\}\\}`, "g"),
			() => token.original,
		);
	}
	return { text: out, missing };
}

function hasNaturalLanguage(text: string): boolean {
	// A paragraph is worth translating only if it contains letters outside
	// placeholders, commands, math, etc.
	const stripped = text.replace(/\{\{\d+\}\}/g, "").replace(/[\\$%{}[\]]/g, "");
	return /[A-Za-z]{3,}/.test(stripped);
}

const MIN_NATURAL_LANG_CHARS = 20;

function isCjkChar(c: string): boolean {
	// CJK Unified Ideographs + Hiragana + Katakana + Hangul Syllables.
	return /[\u4e00-\u9fff\u3040-\u309f\u30a0-\u30ff\uac00-\ud7af]/.test(c);
}

/**
 * Insert `{}` between a command name and a following CJK character so the
 * character is not absorbed as part of the command (e.g. \LaTeX中文).
 *
 * The implementation scans left-to-right and reads the full command name before
 * looking at the next character; this avoids regex backtracking that would
 * otherwise split command names like \documentclass[...] into \documentclas{}s.
 */
export function insertCommandArgSeparators(text: string): string {
	let out = "";
	let i = 0;
	while (i < text.length) {
		if (text[i] === "\\") {
			let j = i + 1;
			while (j < text.length && /[A-Za-z]/.test(text[j])) {
				j++;
			}
			if (j > i + 1) {
				// i..j is a command name such as \documentclass.
				const next = text[j];
				if (next !== undefined && isCjkChar(next)) {
					out += `${text.slice(i, j)}{}`;
					i = j;
					continue;
				}
			}
		}
		out += text[i];
		i++;
	}
	return out;
}

function isWorthTranslating(maskedText: string): boolean {
	const withoutPlaceholders = maskedText.replace(/\{\{\d+\}\}/g, "");
	const letterCount = (withoutPlaceholders.match(/[A-Za-z]/g) ?? []).length;
	if (letterCount < MIN_NATURAL_LANG_CHARS) return false;
	// Avoid translating command-heavy paragraphs where commands dominate prose.
	const nonWhitespace = maskedText.replace(/\s/g, "").length;
	const placeholderChars = (maskedText.match(/\{\{\d+\}\}/g) ?? []).reduce(
		(acc, p) => acc + p.length,
		0,
	);
	const proseChars = nonWhitespace - placeholderChars;
	return proseChars > placeholderChars * 0.5;
}

function splitSentences(text: string): string[] {
	return text
		.replace(/([.!?])\s+/g, "$1\n")
		.split("\n")
		.map((s) => s.trim())
		.filter(Boolean);
}

function chunkByCharLimit(items: string[], maxLen: number): string[][] {
	const chunks: string[][] = [];
	let current: string[] = [];
	let currentLen = 0;
	for (const item of items) {
		if (currentLen + item.length + 1 > maxLen && current.length > 0) {
			chunks.push(current);
			current = [item];
			currentLen = item.length;
		} else {
			current.push(item);
			currentLen += item.length + (current.length > 1 ? 1 : 0);
		}
	}
	if (current.length > 0) chunks.push(current);
	return chunks;
}

function splitIntoTranslatableChunks(
	text: string,
	maxLen = TRANSLATE_CHUNK_CHARS,
): string[] {
	const paragraphs = text
		.split(/\n\s*\n/)
		.map((p) => p.trim())
		.filter(Boolean)
		.filter(hasNaturalLanguage);
	const segments: string[] = [];
	for (const para of paragraphs) {
		if (para.length <= maxLen) {
			segments.push(para);
			continue;
		}
		const sentences = splitSentences(para);
		const chunks = chunkByCharLimit(sentences, maxLen);
		for (const chunk of chunks) segments.push(chunk.join(" "));
	}
	return segments;
}

async function translateChunk(
	text: string,
	run: (text: string) => Promise<string>,
	maxLen: number,
): Promise<string> {
	if (!text.trim()) return text;
	let result = await run(text);
	// If the engine returned the source unchanged, the chunk was likely too long.
	if (result.trim() === text.trim() && text.length > 200) {
		const smaller = splitIntoTranslatableChunks(text, Math.floor(maxLen / 2));
		if (smaller.length > 1) {
			const parts = await Promise.all(
				smaller.map((s) => translateChunk(s, run, maxLen / 2)),
			);
			result = parts.join("\n\n");
		}
	}
	return result;
}

export type TranslateLatexContentOptions = {
	/**
	 * Called with a 0–100 progress value relative to this call's scope.
	 * Callers that process larger documents should wrap this with
	 * {@link scaleProgress} so nested translations report within their slice.
	 */
	onProgress?: (progress: number) => void;
};

/**
 * Wrap a progress callback so that local 0–100 values are mapped to the
 * [start, end] window and never decrease. Returns undefined when the window is
 * empty, which lets callers skip emitting progress for tiny/no-op slices.
 */
function scaleProgress(
	onProgress: ((progress: number) => void) | undefined,
	start: number,
	end: number,
): ((progress: number) => void) | undefined {
	if (!onProgress || end <= start) return undefined;
	let last = -1;
	return (pct: number) => {
		const scaled = start + (pct / 100) * (end - start);
		const rounded = Math.min(100, Math.max(0, Math.round(scaled)));
		if (rounded >= last) {
			last = rounded;
			onProgress(rounded);
		}
	};
}

export async function translateLatexContent(
	text: string,
	runTranslateChunk: (text: string) => Promise<string>,
	opts?: TranslateLatexContentOptions,
): Promise<string> {
	// Translate paragraph-by-paragraph so command-only paragraphs stay intact
	// while prose paragraphs are translated. Global masking per paragraph keeps
	// placeholders local and avoids losing tokens from non-translatable blocks.
	const paragraphs = text.split(/\n\s*\n/);
	const translatedParagraphs: string[] = [];
	let totalMissing = 0;
	const report = (index: number): void => {
		if (!paragraphs.length || !opts?.onProgress) return;
		const pct = Math.round(((index + 1) / paragraphs.length) * 100);
		opts.onProgress(pct);
	};
	for (let i = 0; i < paragraphs.length; i++) {
		const para = paragraphs[i];
		const trimmed = para.trim();
		if (!trimmed) {
			translatedParagraphs.push(para);
			report(i);
			continue;
		}
		const masked = maskLatexSource(trimmed);
		const segments = splitIntoTranslatableChunks(masked.text);
		let translated: string;
		if (segments.length === 0 || !isWorthTranslating(masked.text)) {
			// Command-only or command-heavy paragraph: restore in place.
			const restored = restoreLatexSource(masked.text, masked.tokens);
			translated = insertCommandArgSeparators(restored.text);
			totalMissing += restored.missing;
		} else {
			const parts = await Promise.all(
				segments.map((seg) =>
					translateChunk(seg, runTranslateChunk, TRANSLATE_CHUNK_CHARS),
				),
			);
			const joined = parts.join("\n\n");
			const restored = restoreLatexSource(joined, masked.tokens);
			translated = insertCommandArgSeparators(restored.text);
			totalMissing += restored.missing;
		}
		translatedParagraphs.push(translated);
		report(i);
	}
	if (totalMissing > 0) {
		logger.warn("latex translate: masked tokens lost", { count: totalMissing });
	}
	return translatedParagraphs.join("\n\n");
}

const CAPTION_RE = /\\caption(\*)?\{/g;

/**
 * Translate \caption{...} commands in place. Only the natural-language caption
 * text is translated; any inline math/commands inside the caption are masked.
 */
export async function translateCaptionCommands(
	text: string,
	runTranslateChunk: (text: string) => Promise<string>,
	opts?: TranslateLatexContentOptions,
): Promise<string> {
	let out = "";
	let lastIndex = 0;
	const re = new RegExp(CAPTION_RE.source, CAPTION_RE.flags);
	let m: RegExpExecArray | null = re.exec(text);
	while (m !== null) {
		const start = m.index;
		const braceStart = start + m[0].length - 1; // position of '{'
		const brace = readBalanced(text, braceStart, "{", "}");
		if (!brace) continue;
		const inner = brace.value.slice(1, -1);
		if (!inner.trim() || !hasNaturalLanguage(inner)) {
			lastIndex = brace.end;
			continue;
		}
		const translatedInner = await translateLatexContent(
			inner,
			runTranslateChunk,
			opts,
		);
		out += text.slice(lastIndex, start);
		out += `${m[0].slice(0, -1)}{${translatedInner}}`;
		lastIndex = brace.end;
		m = re.exec(text);
	}
	out += text.slice(lastIndex);
	return out;
}

const SKIP_ENVIRONMENTS = new Set([
	"lstlisting",
	"verbatim",
	"Verbatim",
	"minted",
	"algorithm",
	"algorithmic",
]);

const CAPTION_ENVIRONMENTS = new Set(["figure", "table", "figure*", "table*"]);

function findEnvironmentBoundaries(
	text: string,
	start: number,
): {
	name: string;
	start: number;
	openEnd: number;
	closeStart: number;
	end: number;
} | null {
	const beginMatch = text
		.slice(start)
		.match(/\\begin\{([^}]+)\}(?:\[[^\]]*\])?/);
	if (!beginMatch) return null;
	const name = beginMatch[1];
	const beginIdx = beginMatch.index ?? 0;
	const envStart = start + beginIdx;
	const openEnd = envStart + beginMatch[0].length;
	let depth = 1;
	let i = openEnd;
	while (i < text.length && depth > 0) {
		const rest = text.slice(i);
		const begin = rest.match(/^\\begin\{([^}]+)\}(?:\[[^\]]*\])?/);
		const end = rest.match(/^\\end\{([^}]+)\}/);
		if (begin && begin[1] === name) {
			depth++;
			i += begin[0].length;
		} else if (end && end[1] === name) {
			depth--;
			if (depth === 0) {
				const closeStart = i;
				return {
					name,
					start: envStart,
					openEnd,
					closeStart,
					end: i + end[0].length,
				};
			}
			i += end[0].length;
		} else {
			i++;
		}
	}
	return null;
}

/**
 * Recursively translate a LaTeX document body, skipping verbatim/code
 * environments and translating only captions inside figure/table environments.
 *
 * `opts.onProgress` reports 0–100 for this body only; callers scale it to the
 * file/project level.
 */
export async function translateLatexBody(
	text: string,
	runTranslateChunk: (text: string) => Promise<string>,
	opts?: TranslateLatexContentOptions,
): Promise<string> {
	let out = "";
	let i = 0;
	const total = text.length || 1;
	const report = (pos: number): void => {
		if (!opts?.onProgress) return;
		const pct = Math.min(100, Math.round((pos / total) * 100));
		opts.onProgress(pct);
	};
	while (i < text.length) {
		const env = findEnvironmentBoundaries(text, i);
		if (!env) {
			// No more environments; translate the rest of the text.
			const tail = text.slice(i);
			out += await translateLatexContent(tail, runTranslateChunk, {
				onProgress: scaleProgress(opts?.onProgress, (i / total) * 100, 100),
			});
			report(total);
			break;
		}

		// Translate the text before the environment.
		if (env.start > i) {
			out += await translateLatexContent(
				text.slice(i, env.start),
				runTranslateChunk,
				{
					onProgress: scaleProgress(
						opts?.onProgress,
						(i / total) * 100,
						(env.start / total) * 100,
					),
				},
			);
		}

		const envStartPct = (env.start / total) * 100;
		const envEndPct = (env.end / total) * 100;
		if (SKIP_ENVIRONMENTS.has(env.name)) {
			// Keep the whole environment as-is.
			out += text.slice(env.start, env.end);
		} else if (CAPTION_ENVIRONMENTS.has(env.name)) {
			// Translate captions, keep everything else.
			const envBody = text.slice(env.start, env.end);
			out += await translateCaptionCommands(envBody, runTranslateChunk, {
				onProgress: scaleProgress(opts?.onProgress, envStartPct, envEndPct),
			});
		} else {
			// Recursively translate other environments (abstract, itemize, etc.).
			const inner = text.slice(env.openEnd, env.closeStart);
			const translatedInner = await translateLatexBody(
				inner,
				runTranslateChunk,
				{
					onProgress: scaleProgress(
						opts?.onProgress,
						(env.openEnd / total) * 100,
						(env.closeStart / total) * 100,
					),
				},
			);
			out +=
				text.slice(env.start, env.openEnd) +
				translatedInner +
				text.slice(env.closeStart, env.end);
		}

		i = env.end;
		report(i);
	}
	return out;
}

const INPUT_INCLUDE_RE = /\\(input|include)\{([^}]+)\}/g;

export function rewriteInputPaths(text: string, lang: string): string {
	return text.replace(
		INPUT_INCLUDE_RE,
		(_match, cmd: string, rawPath: string) => {
			const hasExt = rawPath.endsWith(".tex");
			const stem = hasExt ? rawPath.slice(0, -4) : rawPath;
			const translated = `${stem}_${lang}${hasExt ? ".tex" : ""}`;
			return `\\${cmd}{${translated}}`;
		},
	);
}

export function findTexDependencies(text: string, rootDir: string): string[] {
	const deps: string[] = [];
	const re = new RegExp(INPUT_INCLUDE_RE.source, INPUT_INCLUDE_RE.flags);
	let m: RegExpExecArray | null = re.exec(text);
	while (m !== null) {
		const rawPath = m[2];
		const withExt = rawPath.endsWith(".tex") ? rawPath : `${rawPath}.tex`;
		deps.push(joinVaultPath(rootDir, withExt));
		m = re.exec(text);
	}
	return deps;
}

export async function discoverTexFiles(rootTexPath: string): Promise<string[]> {
	const rootDir = dirnameOf(rootTexPath);
	const visited = new Set<string>();
	const files: string[] = [];
	const queue: string[] = [rootTexPath];
	while (queue.length > 0) {
		const current = queue.shift();
		if (current === undefined) break;
		if (visited.has(current)) continue;
		visited.add(current);
		files.push(current);
		let content: string;
		try {
			content = await readVaultFile(current);
		} catch {
			continue;
		}
		for (const dep of findTexDependencies(content, rootDir)) {
			if (!visited.has(dep)) queue.push(dep);
		}
	}
	return files;
}

export function translatedTexPath(texPath: string, lang: string): string {
	const dir = dirnameOf(texPath);
	const base = basenameOf(texPath);
	const stem = base.replace(/\.tex$/, "");
	return joinVaultPath(dir, `${stem}_${lang}.tex`);
}

export function latexTranslationMetaPath(
	paperPath: string,
	lang: string,
): string {
	return joinVaultPath(
		joinVaultPath(paperPath, "source"),
		`.translation_${lang}.meta.json`,
	);
}

export function latexTranslationPdfPath(
	paperPath: string,
	paperId: string,
	lang: string,
): string {
	return joinVaultPath(paperPath, `${paperId}.${lang}.pdf`);
}

export type LatexTranslationMeta = {
	lang: string;
	providerId: string;
	rootTex: string;
	generatedAt: string;
	files: {
		original: string;
		translated: string;
		sourceHash: string;
	}[];
};

async function sha256Text(text: string): Promise<string> {
	const encoder = new TextEncoder();
	const data = encoder.encode(text);
	const hashBuffer = await crypto.subtle.digest("SHA-256", data);
	const hashArray = Array.from(new Uint8Array(hashBuffer));
	return hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");
}

function isCjkLang(lang: string): boolean {
	return lang === "zh-CN" || lang === "zh" || lang === "ja" || lang === "ko";
}

/**
 * Prepare the translated root tex for CJK output:
 * - insert \usepackage{ctex} after \documentclass for Chinese;
 * - drop inputenc/fontenc which conflict with xelatex;
 * - return the engine to use.
 */
export function prepareRootTexForLang(
	text: string,
	lang: string,
): { text: string; engine: "pdflatex" | "xelatex" | "lualatex" } {
	if (!isCjkLang(lang)) {
		return { text, engine: "pdflatex" };
	}

	let out = text;
	// Insert ctex right after \documentclass, but only once.
	if (!/\\usepackage\{ctex\}/.test(out)) {
		out = out.replace(
			/(\\documentclass(?:\[[^\]]*\])?\{[^}]+\})/,
			"$1\n\\usepackage{ctex}",
		);
	}
	// Remove inputenc/fontenc which are incompatible with xelatex/ctex.
	out = out.replace(/\\usepackage\[utf8\]\{inputenc\}\s*/g, "");
	out = out.replace(/\\usepackage\[T1\]\{fontenc\}\s*/g, "");
	// \pdfoutput is a pdftex primitive; xelatex does not define it.
	out = out.replace(/^\\pdfoutput\s*=\s*1\s*\n?/gm, "");
	return { text: out, engine: "xelatex" };
}

export type TranslateLatexProjectOptions = {
	/**
	 * Called with a 0–100 progress value as files are translated.
	 * The value covers the translation phase only (not compilation).
	 */
	onProgress?: (progress: number) => void;
};

export async function translateLatexProject(
	rootTexPath: string,
	lang: string,
	runTranslateChunk: (text: string) => Promise<string>,
	opts?: TranslateLatexProjectOptions,
): Promise<{ rootTranslated: string; meta: LatexTranslationMeta }> {
	const files = await discoverTexFiles(rootTexPath);
	const meta: LatexTranslationMeta = {
		lang,
		providerId: loadSettings().translate.provider,
		rootTex: rootTexPath,
		generatedAt: new Date().toISOString(),
		files: [],
	};

	for (let i = 0; i < files.length; i++) {
		const file = files[i];
		const content = await readVaultFile(file);
		const fileStart = (i / files.length) * 100;
		const fileEnd = ((i + 1) / files.length) * 100;
		const translatedBody = await translateLatexBody(
			content,
			runTranslateChunk,
			{
				onProgress: scaleProgress(opts?.onProgress, fileStart, fileEnd),
			},
		);
		const rewritten = rewriteInputPaths(translatedBody, lang);
		const outPath = translatedTexPath(file, lang);
		await writeVaultFile(outPath, rewritten);
		meta.files.push({
			original: file,
			translated: outPath,
			sourceHash: await sha256Text(content),
		});
		opts?.onProgress?.(Math.round(fileEnd));
	}

	const rootTranslated = translatedTexPath(rootTexPath, lang);
	return { rootTranslated, meta };
}

export async function writeLatexTranslationMeta(
	paperPath: string,
	lang: string,
	meta: LatexTranslationMeta,
): Promise<void> {
	await writeVaultFile(
		latexTranslationMetaPath(paperPath, lang),
		`${JSON.stringify(meta, null, 2)}\n`,
	);
}

export async function readLatexTranslationMeta(
	paperPath: string,
	lang: string,
): Promise<LatexTranslationMeta | null> {
	try {
		const raw = await readVaultFile(latexTranslationMetaPath(paperPath, lang));
		return JSON.parse(raw) as LatexTranslationMeta;
	} catch {
		return null;
	}
}

export function createLatexTranslateRunner(
	opts: TranslateRunOptions = {},
): (text: string) => Promise<string> {
	const settings = loadSettings();
	const langs = langsFromSettings(settings.translate, i18n.language ?? "en");
	return async (text: string) => {
		const result = await runTranslate(
			{
				text,
				sourceLang: langs.sourceLang,
				targetLang: langs.targetLang,
				context: { surface: "latex-source" },
			},
			opts,
		);
		return result;
	};
}

export function latexTranslateErrorMessage(e: unknown): string {
	return errorText(e);
}
