import type { PdfEngine } from "@embedpdf/models";
import {
	Clock,
	Highlighter,
	Languages,
	Library,
	Loader2,
	ScanSearch,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { PDF_CHROME_CHIP } from "@/components/viewer/pdf/chrome/pdf-chrome-surface";
import i18n from "@/i18n";
import { cn } from "@/lib/core/utils";
import { formatShortcutById } from "@/lib/shell/shortcuts";

type PdfToolbarProps = {
	regionSelecting: boolean;
	visualCropPending: boolean;
	engine: PdfEngine | null;
	onToggleRegionSelect: () => void;
	layoutTranslateRunning: boolean;
	/** Queued behind layout analysis; plain click cancels the wait. */
	layoutTranslateWaiting?: boolean;
	layoutTranslateActive: boolean;
	layoutTranslateLabel: string;
	layoutTranslateProgress?: {
		total: number;
		done: number;
		error: number;
	};
	onToggleLayoutTranslate: () => void;
	/** True while jEV smart highlights are being generated. */
	smartHighlightBusy?: boolean;
	/** Trigger jEV smart highlighting for the current paper. */
	onSmartHighlight?: () => void;
	/** True while a LaTeX-source translation is running for this paper. */
	latexTranslateRunning?: boolean;
	/** True when viewing a remote paper that has no local sidecar. */
	isRemotePaper?: boolean;
	/** Import the remote paper into the current vault. */
	onImportToLibrary?: () => void;
	/** True while the import is running. */
	importBusy?: boolean;
};

/** Top-right toolbar: region select, bulk translate. Always visible. */
export function PdfToolbar({
	regionSelecting,
	visualCropPending,
	engine,
	onToggleRegionSelect,
	layoutTranslateRunning,
	layoutTranslateWaiting = false,
	layoutTranslateActive,
	layoutTranslateLabel,
	layoutTranslateProgress,
	onToggleLayoutTranslate,
	smartHighlightBusy = false,
	onSmartHighlight,
	latexTranslateRunning = false,
	isRemotePaper = false,
	onImportToLibrary,
	importBusy = false,
}: PdfToolbarProps) {
	const { t } = useTranslation("viewer");
	const layoutTranslateProgressLabel =
		layoutTranslateProgress && layoutTranslateProgress.total > 0
			? `${t("pdf.layoutTranslate.progress", {
					done: layoutTranslateProgress.done,
					total: layoutTranslateProgress.total,
				})}${
					layoutTranslateProgress.error > 0
						? ` · ${t("pdf.layoutTranslate.failedCount", {
								count: layoutTranslateProgress.error,
							})}`
						: ""
				}`
			: null;
	const layoutTranslateAriaLabel = layoutTranslateProgressLabel
		? `${layoutTranslateLabel} · ${layoutTranslateProgressLabel}`
		: layoutTranslateLabel;

	const LONG_PRESS_MS = 300;
	const longPressTimerRef = useRef<number | null>(null);
	const longPressTriggeredRef = useRef(false);
	const suppressNextClickRef = useRef(false);
	const [longPressing, setLongPressing] = useState(false);

	const clearLongPressTimer = useCallback(() => {
		if (longPressTimerRef.current != null) {
			clearTimeout(longPressTimerRef.current);
			longPressTimerRef.current = null;
		}
	}, []);

	useEffect(() => clearLongPressTimer, [clearLongPressTimer]);

	const anyTranslateRunning = layoutTranslateRunning || latexTranslateRunning;

	const handleTranslatePointerDown = useCallback(
		(event: React.PointerEvent<HTMLButtonElement>) => {
			if (event.button !== 0) return;
			if (layoutTranslateActive || anyTranslateRunning) return;
			if (layoutTranslateWaiting) return;
			setLongPressing(true);
			longPressTriggeredRef.current = false;
			suppressNextClickRef.current = false;
			longPressTimerRef.current = window.setTimeout(() => {
				longPressTriggeredRef.current = true;
				suppressNextClickRef.current = true;
				setLongPressing(true);
				onToggleLayoutTranslate();
			}, LONG_PRESS_MS);
		},
		[
			layoutTranslateActive,
			anyTranslateRunning,
			layoutTranslateWaiting,
			onToggleLayoutTranslate,
		],
	);

	const handleTranslatePointerUp = useCallback(
		(event: React.PointerEvent<HTMLButtonElement>) => {
			clearLongPressTimer();
			setLongPressing(false);
			if (event.button !== 0) return;
			if (!longPressTriggeredRef.current) return;
			longPressTriggeredRef.current = false;
			if (layoutTranslateActive || layoutTranslateRunning) {
				onToggleLayoutTranslate();
			}
		},
		[
			clearLongPressTimer,
			layoutTranslateActive,
			layoutTranslateRunning,
			onToggleLayoutTranslate,
		],
	);

	const handleTranslatePointerLeave = useCallback(() => {
		clearLongPressTimer();
		setLongPressing(false);
		if (!longPressTriggeredRef.current) return;
		longPressTriggeredRef.current = false;
		suppressNextClickRef.current = true;
		if (layoutTranslateActive || layoutTranslateRunning) {
			onToggleLayoutTranslate();
		}
	}, [
		clearLongPressTimer,
		layoutTranslateActive,
		layoutTranslateRunning,
		onToggleLayoutTranslate,
	]);

	const handleTranslateClick = useCallback(
		(event: React.MouseEvent<HTMLButtonElement>) => {
			if (suppressNextClickRef.current) {
				event.preventDefault();
				suppressNextClickRef.current = false;
				return;
			}
			if (latexTranslateRunning) return;
			onToggleLayoutTranslate();
		},
		[latexTranslateRunning, onToggleLayoutTranslate],
	);

	return (
		<div className="pointer-events-none absolute top-2 right-3 z-20 flex origin-top-right items-center gap-1">
			<TooltipProvider delayDuration={200}>
				<div
					data-pdf-chrome
					className={cn(
						"pointer-events-auto flex h-7 select-none items-center gap-0.5 rounded-lg p-0.5",
						PDF_CHROME_CHIP,
					)}
				>
					{isRemotePaper ? (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									type="button"
									size="icon-xs"
									variant="ghost"
									className="shrink-0 self-center"
									aria-label={t("pdf.importToLibrary")}
									disabled={importBusy}
									onClick={onImportToLibrary}
								>
									{importBusy ? (
										<Loader2 className="size-3.5 animate-spin" aria-hidden />
									) : (
										<Library className="size-3.5" aria-hidden />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent side="bottom">
								{t("pdf.importToLibrary")}
							</TooltipContent>
						</Tooltip>
					) : null}
					{!isRemotePaper ? (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									type="button"
									size="icon-xs"
									variant={regionSelecting ? "secondary" : "ghost"}
									className="shrink-0 self-center"
									aria-label={t("pdfExplain.selectRegion")}
									aria-pressed={regionSelecting}
									disabled={visualCropPending || !engine}
									onClick={onToggleRegionSelect}
								>
									<ScanSearch
										className={cn(
											"size-3.5",
											visualCropPending && "animate-pulse",
										)}
									/>
								</Button>
							</TooltipTrigger>
							<TooltipContent side="bottom">
								{regionSelecting
									? t("pdfExplain.cancelRegion")
									: t("pdfExplain.selectRegion")}
								{/* Inverted tooltip: mute via text-background, not muted-foreground. */}
								<span className="ml-2 text-background/70">
									{formatShortcutById("visualAnnotation")}
								</span>
							</TooltipContent>
						</Tooltip>
					) : null}
					{!isRemotePaper ? (
						<>
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										type="button"
										size="icon-xs"
										variant={
											layoutTranslateActive ||
											layoutTranslateWaiting ||
											longPressing
												? "secondary"
												: "ghost"
										}
										className="shrink-0 self-center"
										data-full-text-translate
										aria-label={
											latexTranslateRunning
												? i18n.t("viewer:pdf.latexTranslation.translating")
												: layoutTranslateAriaLabel
										}
										aria-pressed={
											layoutTranslateActive ||
											layoutTranslateWaiting ||
											latexTranslateRunning
										}
										disabled={!engine || latexTranslateRunning}
										onPointerDown={handleTranslatePointerDown}
										onPointerUp={handleTranslatePointerUp}
										onPointerLeave={handleTranslatePointerLeave}
										onPointerCancel={handleTranslatePointerLeave}
										onClick={handleTranslateClick}
									>
										{layoutTranslateWaiting ? (
											<Clock className="size-3.5 animate-pulse" aria-hidden />
										) : anyTranslateRunning && !longPressing ? (
											<Loader2 className="size-3.5 animate-spin" aria-hidden />
										) : (
											<Languages className="size-3.5" aria-hidden />
										)}
									</Button>
								</TooltipTrigger>
								<TooltipContent side="bottom">
									{latexTranslateRunning
										? i18n.t("viewer:pdf.latexTranslation.translating")
										: layoutTranslateLabel}
									{!latexTranslateRunning && layoutTranslateProgressLabel ? (
										<span className="ml-1 text-background/80">
											· {layoutTranslateProgressLabel}
										</span>
									) : null}
									{/* Inverted tooltip: mute via text-background, not muted-foreground. */}
									<span className="ml-2 text-background/70">
										{formatShortcutById("layoutTranslate")}
									</span>
								</TooltipContent>
							</Tooltip>
							{!latexTranslateRunning && layoutTranslateProgressLabel ? (
								<span
									data-full-text-translate-progress
									className="max-w-44 truncate px-1 text-[10px] tabular-nums text-muted-foreground"
								>
									{layoutTranslateProgressLabel}
								</span>
							) : null}
						</>
					) : null}
					{!isRemotePaper && onSmartHighlight ? (
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									type="button"
									size="icon-xs"
									variant="ghost"
									className="shrink-0 self-center"
									aria-label={t("pdf.smartHighlight")}
									disabled={!engine || smartHighlightBusy}
									onClick={onSmartHighlight}
								>
									{smartHighlightBusy ? (
										<Loader2 className="size-3.5 animate-spin" aria-hidden />
									) : (
										<Highlighter className="size-3.5" aria-hidden />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent side="bottom">
								{t("pdf.smartHighlight")}
							</TooltipContent>
						</Tooltip>
					) : null}
				</div>
			</TooltipProvider>
		</div>
	);
}
