import { describe, expect, it } from "vitest";

import { maskInlineTokens, restoreInlineTokens } from "@/lib/translate/mask";

/**
 * Minimal prototype of a LaTeX prose extractor for issue #591 testing.
 * It pulls natural-language strings out of common document commands and
 * environments, leaving commands/environments intact.
 */
type LatexSegment = {
	id: string;
	surface:
		| "title"
		| "author"
		| "section"
		| "abstract"
		| "paragraph"
		| "caption";
	source: string;
	parentCommand?: string;
};

function extractLatexTranslatableSegments(latex: string): LatexSegment[] {
	const segments: LatexSegment[] = [];
	let id = 0;
	const nextId = () => `s${id++}`;

	// \title{...}, \author{...}, \section{...}, \subsection{...}, \caption{...}
	const commandTextRe =
		/\\(title|author|section|subsection|subsubsection|caption)\*?\{([\s\S]*?)\}/g;
	for (const m of latex.matchAll(commandTextRe)) {
		const cmd = m[1] as LatexSegment["surface"];
		const inner = m[2].trim();
		if (inner) {
			segments.push({
				id: nextId(),
				surface: cmd === "caption" ? "caption" : cmd,
				source: inner,
				parentCommand: `\\${cmd}`,
			});
		}
	}

	// \begin{abstract}...\end{abstract}
	const abstractRe = /\\begin\{abstract\}([\s\S]*?)\\end\{abstract\}/g;
	for (const m of latex.matchAll(abstractRe)) {
		const inner = m[1].trim();
		if (inner) {
			segments.push({
				id: nextId(),
				surface: "abstract",
				source: inner,
			});
		}
	}

	// Plain paragraphs in the document body.
	// Strategy: line-by-line, drop command/environment/comment lines, then group
	// consecutive non-empty text lines into paragraphs.
	const bodyStart = latex.indexOf("\\begin{document}");
	const body = bodyStart >= 0 ? latex.slice(bodyStart) : latex;
	const lines = body.split("\n");
	const textLines: string[] = [];
	for (const rawLine of lines) {
		const line = rawLine.trim();
		if (!line) continue;
		if (line.startsWith("\\")) continue;
		if (line.startsWith("%")) continue;
		textLines.push(line);
	}

	// A text line that already came from a captured command argument is skipped
	// so we do not duplicate it.
	const captured = new Set(segments.map((s) => s.source));
	const pending: string[] = [];
	const flushParagraph = () => {
		if (pending.length === 0) return;
		const cleaned = pending.join(" ").trim();
		pending.length = 0;
		if (!cleaned || captured.has(cleaned)) return;
		segments.push({
			id: nextId(),
			surface: "paragraph",
			source: cleaned,
		});
	};
	for (const line of textLines) {
		if (captured.has(line)) {
			flushParagraph();
			continue;
		}
		pending.push(line);
	}
	flushParagraph();

	return segments;
}

/** Simulate an MT engine that only touches plain words. */
function mockTranslate(text: string): string {
	return text
		.replace(/\bWe propose\b/g, "我们提出")
		.replace(/\battention\b/g, "注意力")
		.replace(/\bTransformer\b/g, "Transformer")
		.replace(/\brecurrent\b/g, "循环")
		.replace(/\bconvolutional\b/g, "卷积")
		.replace(/\bneural networks\b/g, "神经网络");
}

describe("latex translate mask roundtrip", () => {
	const sampleLatex = `\\documentclass{article}
\\usepackage{amsmath}
\\begin{document}
\\title{Attention Is All You Need}
\\author{Ashish Vaswani}
\\maketitle

\\begin{abstract}
We propose a new simple network architecture, the Transformer, based solely on attention mechanisms.
\\end{abstract}

\\section{Introduction}
The dominant sequence transduction models are based on complex recurrent or convolutional neural networks.

We define $\\mathbf{Q} \\in \\mathbb{R}^{n \\times d_k}$ and compute attention as:
\\begin{equation}
\\mathrm{Attention}(Q, K, V) = \\mathrm{softmax}\\left(\\frac{QK^T}{\\sqrt{d_k}}\\right)V
\\end{equation}

See \\cite{vaswani2017attention} for details.
\\end{document}
`;

	it("extracts translatable segments from sample latex", () => {
		const segments = extractLatexTranslatableSegments(sampleLatex);
		const sources = segments.map((s) => s.source);
		expect(sources).toContain("Attention Is All You Need");
		expect(sources).toContain("Ashish Vaswani");
		expect(sources).toContain("Introduction");
		expect(sources).toContain(
			"We propose a new simple network architecture, the Transformer, based solely on attention mechanisms.",
		);
		// The naive extractor groups consecutive text lines; the paragraph after
		// \section{Introduction} is merged with the following text lines.
		const paragraph = segments.find((s) => s.surface === "paragraph");
		expect(paragraph?.source).toContain(
			"The dominant sequence transduction models are based on complex recurrent or convolutional neural networks.",
		);
	});

	it("masks inline math and simple commands before translation", () => {
		const segment =
			"We define $\\mathbf{Q} \\in \\mathbb{R}^{n \\times d_k}$ and compute attention.";
		const masked = maskInlineTokens(segment);
		expect(masked.text).not.toContain("$\\mathbf{Q}");
		expect(masked.text).not.toContain("\\mathbb{R}");
		expect(masked.tokens.length).toBeGreaterThan(0);
		expect(masked.tokens[0]?.original.startsWith("$")).toBe(true);
	});

	it("restores masked tokens after mock translation", () => {
		const segment =
			"We define $\\mathbf{Q} \\in \\mathbb{R}^{n \\times d_k}$ and compute attention.";
		const masked = maskInlineTokens(segment);
		const translated = mockTranslate(masked.text);
		const restored = restoreInlineTokens(translated, masked.tokens);
		expect(restored.text).toContain(
			"$\\mathbf{Q} \\in \\mathbb{R}^{n \\times d_k}$",
		);
		expect(restored.text).toContain("注意力");
		expect(restored.missing).toBe(0);
	});

	it("round-trips a full latex segment through mask-translate-restore", () => {
		const segments = extractLatexTranslatableSegments(sampleLatex);
		const paragraph = segments.find(
			(s) => s.surface === "paragraph" && s.source.includes("$\\mathbf{Q}"),
		);
		expect(paragraph).toBeDefined();
		if (!paragraph) return;

		const masked = maskInlineTokens(paragraph.source);
		const translated = mockTranslate(masked.text);
		const restored = restoreInlineTokens(translated, masked.tokens);

		expect(restored.text).toContain(
			"$\\mathbf{Q} \\in \\mathbb{R}^{n \\times d_k}$",
		);
		expect(restored.text).toContain("注意力");
		expect(restored.missing).toBe(0);
	});

	it("masks nested math commands including \\frac and \\left", () => {
		const nested =
			"\\mathrm{Attention}(Q, K, V) = \\mathrm{softmax}\\left(\\frac{QK^T}{\\sqrt{d_k}}\\right)V";
		const masked = maskInlineTokens(nested);

		// All LaTeX commands are replaced by placeholders; the translator only sees
		// the placeholder brackets and literal braces that structure \\frac.
		expect(masked.text).not.toContain("\\mathrm");
		expect(masked.text).not.toContain("\\left");
		expect(masked.text).not.toContain("\\frac");
		expect(masked.text).not.toContain("\\sqrt");
		expect(masked.text).not.toContain("\\right");
		expect(masked.tokens.length).toBeGreaterThanOrEqual(4);
	});

	it("masks display environment markers and nested math", () => {
		const display =
			"\\begin{equation}\n\\mathrm{Attention}(Q, K, V) = \\mathrm{softmax}\\left(\\frac{QK^T}{\\sqrt{d_k}}\\right)V\n\\end{equation}";
		const masked = maskInlineTokens(display);

		// Environment markers and all \\cmd sequences become placeholders.
		expect(masked.text).not.toContain("\\begin{equation}");
		expect(masked.text).not.toContain("\\end{equation}");
		expect(masked.text).not.toContain("\\mathrm");
		expect(masked.text).not.toContain("\\left");
		expect(masked.text).not.toContain("\\frac");
		expect(masked.text).not.toContain("\\sqrt");
		expect(masked.text).not.toContain("\\right");
		expect(masked.tokens.length).toBeGreaterThanOrEqual(6);
	});

	it("restores nested math after mock translation", () => {
		const display =
			"\\begin{equation}\n\\mathrm{Attention}(Q, K, V) = \\mathrm{softmax}\\left(\\frac{QK^T}{\\sqrt{d_k}}\\right)V\n\\end{equation}";
		const masked = maskInlineTokens(display);
		// Simulate an MT engine that does not touch placeholders but may add spaces.
		const translated = masked.text.replace(/Attention/g, "注意力");
		const restored = restoreInlineTokens(translated, masked.tokens);

		expect(restored.text).toContain("\\begin{equation}");
		expect(restored.text).toContain("\\end{equation}");
		expect(restored.text).toContain("\\mathrm{Attention}");
		expect(restored.text).toContain("\\mathrm{softmax}");
		expect(restored.text).toContain("\\left(");
		expect(restored.text).toContain("\\frac{QK^T}{\\sqrt{d_k}}");
		expect(restored.text).toContain("\\right)V");
		expect(restored.missing).toBe(0);
	});
});
