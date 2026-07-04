import React, { useCallback, useRef, useLayoutEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useOCR } from '@/Manatan/context/OCRContext';
import { cleanPunctuation, lookupYomitan } from '@/Manatan/utils/api';
import { DictionaryView } from '@/Manatan/components/DictionaryView';
import { DictionaryResult } from '@/Manatan/types';
import { buildScopedCustomCss } from '@/Manatan/utils/customCss';
import { getPopupTheme } from '@/features/ln/reader/utils/themes';

const POPUP_GAP = 10;
const POPUP_MIN_WIDTH_PX = 280;
const POPUP_MAX_WIDTH_PX = 1920;
const POPUP_MIN_HEIGHT_PX = 200;
const POPUP_MAX_HEIGHT_PX = 1080;

const STRUCTURED_CONTENT_CSS = `
.yomitan-popup {
    --text-color: currentColor;
    --background-color: transparent;
    --link-color: #7cc8ff;
    --accent-color: #7cc8ff;
    --light-border-color: rgba(160, 160, 160, 0.42);
    --medium-border-color: rgba(160, 160, 160, 0.62);
}

.yomitan-popup [data-sc-class~="tag"],
.yomitan-popup .tag,
.yomitan-popup .tag-label {
    display: inline-flex !important;
    align-items: center;
    max-width: 100%;
    margin: 0.12em 0.35em 0.12em 0 !important;
    padding: 0.18em 0.42em !important;
    border-radius: 0.35em !important;
    font-size: 0.78em !important;
    font-weight: 700;
    line-height: 1.25 !important;
    vertical-align: baseline;
    white-space: normal !important;
    overflow-wrap: anywhere !important;
    word-break: normal;
}

.yomitan-popup .tag > .tag-label {
    display: inline !important;
    margin: 0 !important;
    padding: 0 !important;
    border-radius: 0 !important;
    background-color: transparent !important;
    color: inherit !important;
    font-size: inherit !important;
    line-height: inherit !important;
    white-space: inherit !important;
}

.yomitan-popup .tag-list,
.yomitan-popup .definition-tag-list,
.yomitan-popup .headword-list-tag-list {
    display: flex !important;
    flex-wrap: wrap;
    gap: 0.28rem;
    min-width: 0;
}

.yomitan-popup .definition-item,
.yomitan-popup .gloss-item {
    gap: 0.55rem;
    min-width: 0;
}

.yomitan-popup .definition-item > *,
.yomitan-popup .gloss-item > * {
    min-width: 0;
}

.yomitan-popup [data-sc-content="part-of-speech-info"],
.yomitan-popup .tag-part-of-speech {
    background-color: #565656 !important;
    color: #fff !important;
}

.yomitan-popup [data-sc-content="misc-info"] {
    background-color: #7a4638 !important;
    color: #fff !important;
}

.yomitan-popup [data-sc-content="field-info"] {
    background-color: #73509d !important;
    color: #fff !important;
}

.yomitan-popup [data-sc-content="dialect-info"] {
    background-color: #3d7b4f !important;
    color: #fff !important;
}

.yomitan-popup [data-sc-content="sense-group"],
.yomitan-popup [data-sc-content="sense"],
.yomitan-popup [data-sc-role="sense-item"] {
    margin-top: 0.25em;
}

.yomitan-popup [data-sc-content="glossary"],
.yomitan-popup [data-sc-role="pattern-list"] {
    margin: 0.2em 0 0.35em;
    padding-left: 1.15em;
}

.yomitan-popup [data-sc-class~="extra-box"],
.yomitan-popup [data-sc-role="block-body"] {
    border-left: 3px solid rgba(150, 150, 150, 0.7) !important;
    border-radius: 0.4rem !important;
    margin: 0.55rem 0 !important;
    padding: 0.5rem 0.65rem !important;
    background-color: rgba(150, 150, 150, 0.08) !important;
}

.yomitan-popup [data-sc-content="example-sentence"] {
    border-left-color: rgba(170, 170, 170, 0.86) !important;
    background-color: rgba(170, 170, 170, 0.09) !important;
}

.yomitan-popup [data-sc-content="sense-note"],
.yomitan-popup [data-sc-content="info-gloss"],
.yomitan-popup [data-sc-role="block-body"][data-sc-kind="explains"] {
    border-left-color: #c89b32 !important;
    background-color: rgba(200, 155, 50, 0.11) !important;
}

.yomitan-popup [data-sc-content="lang-source"],
.yomitan-popup [data-sc-content="keyword"],
.yomitan-popup [data-sc-role="block-body"][data-sc-kind="keyword"] {
    border-left-color: #9b6bd3 !important;
    background-color: rgba(155, 107, 211, 0.11) !important;
}

.yomitan-popup [data-sc-content="xref"] {
    border-left-color: #4f9cff !important;
    background-color: rgba(79, 156, 255, 0.11) !important;
}

.yomitan-popup [data-sc-content="antonym"],
.yomitan-popup [data-sc-role="block-body"][data-sc-kind="examples"] {
    border-left-color: #d46a6a !important;
    background-color: rgba(212, 106, 106, 0.11) !important;
}

.yomitan-popup [data-sc-content="example-sentence-a"] {
    font-size: 1.12em;
}

.yomitan-popup [data-sc-content="example-sentence-b"],
.yomitan-popup [data-sc-content="xref-glossary"],
.yomitan-popup [data-sc-content="antonym-glossary"],
.yomitan-popup [data-sc-class="extra-label"] {
    color: var(--text-muted, rgba(180, 180, 180, 0.88));
    font-size: 0.9em;
}

.yomitan-popup ruby rt {
    visibility: visible !important;
    opacity: 0.82 !important;
    font-size: 0.62em !important;
}

@media (pointer: coarse), (max-width: 700px) {
    .yomitan-popup {
        font-size: 15px;
        line-height: 1.55;
        overflow-wrap: break-word;
        word-break: normal;
    }

    .yomitan-popup [data-sc-class~="tag"],
    .yomitan-popup .tag,
    .yomitan-popup .tag-label {
        margin: 0.16em 0.4em 0.16em 0 !important;
        padding: 0.22em 0.48em !important;
    }

    .yomitan-popup .tag > .tag-label {
        margin: 0 !important;
        padding: 0 !important;
    }

    .yomitan-popup [data-sc-class~="extra-box"],
    .yomitan-popup [data-sc-role="block-body"] {
        margin: 0.5rem 0;
        padding: 0.5rem 0.6rem;
    }

    .yomitan-popup .definition-item,
    .yomitan-popup .gloss-item {
        align-items: flex-start;
    }
}
`;


interface LookupHistoryEntry {
    term: string;
    results: DictionaryResult[];
    kanjiResults: any[];
    isLoading: boolean;
    systemLoading: boolean;
    isKanjiOnly?: boolean;
}

const HighlightOverlay = () => {
    const { dictPopup } = useOCR();
    if (!dictPopup.visible || !dictPopup.highlight?.rects) return null;

    return (
        <div
            className="dictionary-highlight-overlay"
            style={{
                position: 'fixed',
                top: 0,
                left: 0,
                width: '100%',
                height: '100%',
                pointerEvents: 'none',
                zIndex: 2147483645
            }}
        >
            {dictPopup.highlight.rects.map((rect, i) => (
                <div
                    key={i}
                    style={{
                        position: 'fixed',
                        left: rect.x,
                        top: rect.y,
                        width: rect.width,
                        height: rect.height,
                        backgroundColor: 'rgba(120, 120, 120, 0.28)',
                        borderRadius: '2px',
                        borderBottom: '2px solid rgba(95, 95, 95, 0.85)',
                    }}
                />
            ))}
        </div>
    );
};

export const YomitanPopup = () => {
    const { dictPopup, setDictPopup, notifyPopupClosed, settings } = useOCR();
    const popupRef = useRef<HTMLDivElement>(null);
    const backdropRef = useRef<HTMLDivElement>(null);
    const openedAtRef = useRef(0);
    const [posStyle, setPosStyle] = React.useState<React.CSSProperties>({});
    const [history, setHistory] = useState<LookupHistoryEntry[]>([]);
    const [historyIndex, setHistoryIndex] = useState(-1);

    // Handle native Android back button
    React.useEffect(() => {
        const handleNativeBack = () => {
            if (dictPopup.visible) {
                window.getSelection()?.removeAllRanges();
                notifyPopupClosed();
                setDictPopup(prev => ({ ...prev, visible: false }));
                return true;
            }
            return false;
        };

        // @ts-ignore - global function for Android native bridge
        window.__handleNativeBack = handleNativeBack;

        return () => {
            // @ts-ignore
            window.__handleNativeBack = undefined;
        };
    }, [dictPopup.visible, setDictPopup, notifyPopupClosed]);

    const maxHistory = settings.yomitanLookupMaxHistory || 10;
    const navMode = settings.yomitanLookupNavigationMode || 'stacked';

    const currentEntry = historyIndex >= 0 && historyIndex < history.length ? history[historyIndex] : null;

    const popupWidthPxRaw = Number.isFinite(settings.yomitanPopupWidthPx)
        ? settings.yomitanPopupWidthPx
        : 340;
    const popupHeightPxRaw = Number.isFinite(settings.yomitanPopupHeightPx)
        ? settings.yomitanPopupHeightPx
        : 450;
    const popupScaleRaw = Number.isFinite(settings.yomitanPopupScalePercent)
        ? settings.yomitanPopupScalePercent
        : 100;
    const popupScalePercent = Math.min(Math.max(popupScaleRaw, 50), 200);
    const popupScale = popupScalePercent / 100;
    const popupWidthPx = Math.min(Math.max(popupWidthPxRaw, POPUP_MIN_WIDTH_PX), POPUP_MAX_WIDTH_PX);
    const popupHeightPx = Math.min(Math.max(popupHeightPxRaw, POPUP_MIN_HEIGHT_PX), POPUP_MAX_HEIGHT_PX);
    const popupWidthStyle = `${popupWidthPx}px`;
    const customPopupCss = React.useMemo(
        () => buildScopedCustomCss(settings.yomitanPopupCustomCss, '.yomitan-popup'),
        [settings.yomitanPopupCustomCss],
    );

    const processedEntries = currentEntry ? currentEntry.results : dictPopup.results;
    const kanjiResults = currentEntry ? currentEntry.kanjiResults : (dictPopup.kanjiResults || []);
    const isLoading = currentEntry ? currentEntry.isLoading : dictPopup.isLoading;
    const systemLoading = currentEntry ? currentEntry.systemLoading : dictPopup.systemLoading ?? false;

    React.useEffect(() => {
        if (dictPopup.visible) {
            openedAtRef.current = Date.now();
        }
    }, [dictPopup.visible]);

    // Sync history when initial lookup completes
    React.useEffect(() => {
        if (!dictPopup.visible) {
            setHistory([]);
            setHistoryIndex(-1);
            return;
        }
        if ((dictPopup.results.length > 0 || (dictPopup.kanjiResults && dictPopup.kanjiResults.length > 0)) && !dictPopup.isLoading) {
            const term = dictPopup.results[0]?.headword || dictPopup.kanjiResults?.[0]?.character || dictPopup.context?.sentence?.trim() || '';
            setHistory([{
                term,
                results: dictPopup.results,
                kanjiResults: dictPopup.kanjiResults || [],
                isLoading: false,
                systemLoading: false
            }]);
            setHistoryIndex(0);
        }
    }, [dictPopup.visible, dictPopup.results, dictPopup.kanjiResults, dictPopup.isLoading, dictPopup.systemLoading, dictPopup.context?.sentence]);

    const handleDefinitionLink = useCallback(async (href: string, text: string) => {
        // Extract lookup text from href
        const safeFallback = text.trim();
        const trimmedHref = href.trim();
        let lookupText = safeFallback;

        if (trimmedHref) {
            const extractQuery = (params: URLSearchParams) =>
                params.get('query') || params.get('text') || params.get('term') || params.get('q') || '';

            if (trimmedHref.startsWith('http://') || trimmedHref.startsWith('https://')) {
                try {
                    const parsed = new URL(trimmedHref);
                    const queryText = extractQuery(parsed.searchParams);
                    if (queryText) lookupText = queryText;
                } catch (err) {
                    console.warn('Failed to parse http link', err);
                }
            } else if (trimmedHref.startsWith('?') || trimmedHref.includes('?')) {
                const queryString = trimmedHref.startsWith('?')
                    ? trimmedHref.slice(1)
                    : trimmedHref.slice(trimmedHref.indexOf('?') + 1);
                const params = new URLSearchParams(queryString);
                const queryText = extractQuery(params);
                if (queryText) lookupText = queryText;
            } else if (trimmedHref.startsWith('term://')) {
                lookupText = decodeURIComponent(trimmedHref.slice('term://'.length));
            } else if (trimmedHref.startsWith('yomitan://')) {
                try {
                    const parsed = new URL(trimmedHref);
                    const queryText = extractQuery(parsed.searchParams);
                    if (queryText) lookupText = queryText;
                    else lookupText = decodeURIComponent(parsed.pathname.replace(/^\//, ''));
                } catch (err) {
                    console.warn('Failed to parse yomitan link', err);
                }
            } else {
                try {
                    lookupText = decodeURIComponent(trimmedHref);
                } catch (err) {
                    lookupText = safeFallback || trimmedHref;
                }
            }
        }

        const cleanText = cleanPunctuation(lookupText, true).trim();
        if (!cleanText) return;

        const newEntry: LookupHistoryEntry = {
            term: cleanText,
            results: [],
            kanjiResults: [],
            isLoading: true,
            systemLoading: false
        };

        if (navMode === 'tabs') {
            setHistory(prev => {
                const newHistory = prev.slice(0, historyIndex + 1);
                newHistory.push(newEntry);
                if (newHistory.length > maxHistory) newHistory.shift();
                return newHistory;
            });
            setHistoryIndex(prev => Math.min(prev + 1, maxHistory - 1));
        } else {
            setHistory(prev => {
                const newHistory = prev.slice(0, historyIndex + 1);
                newHistory.push(newEntry);
                if (newHistory.length > maxHistory) newHistory.shift();
                return newHistory;
            });
            setHistoryIndex(prev => Math.min(prev + 1, maxHistory - 1));
        }

        try {
            const results = await lookupYomitan(cleanText, 0, settings.resultGroupingMode || 'grouped', settings.yomitanLanguage || 'japanese');
            const loadedResults = results === 'loading' ? [] : ((results as any).terms || results || []);
            const loadedKanji = results === 'loading' ? [] : ((results as any).kanji || []);
            const isSystemLoading = results === 'loading';

            setHistory(prev => {
                const newHistory = [...prev];
                const idx = Math.min(historyIndex + 1, maxHistory - 1);
                if (newHistory[idx]) {
                    newHistory[idx] = {
                        ...newHistory[idx],
                        results: loadedResults,
                        kanjiResults: loadedKanji,
                        isLoading: false,
                        systemLoading: isSystemLoading
                    };
                }
                return newHistory;
            });
        } catch (err) {
            console.warn('Failed to lookup link definition', err);
            setHistory(prev => {
                const newHistory = [...prev];
                const idx = Math.min(historyIndex + 1, maxHistory - 1);
                if (newHistory[idx]) {
                    newHistory[idx] = { ...newHistory[idx], results: [], isLoading: false, systemLoading: false };
                }
                return newHistory;
            });
        }
    }, [setDictPopup, navMode, historyIndex, maxHistory]);

    const handleWordClick = useCallback(async (text: string, position: number) => {
        const textEncoder = new TextEncoder();
        const prefixBytes = textEncoder.encode(text.slice(0, position)).length;
        
        const cleanText = cleanPunctuation(text, true).trim();
        if (!cleanText) return;

        const newEntry: LookupHistoryEntry = {
            term: cleanText,
            results: [],
            kanjiResults: [],
            isLoading: true,
            systemLoading: false
        };

        if (navMode === 'tabs') {
            setHistory(prev => {
                const newHistory = prev.slice(0, historyIndex + 1);
                newHistory.push(newEntry);
                if (newHistory.length > maxHistory) newHistory.shift();
                return newHistory;
            });
            setHistoryIndex(prev => Math.min(prev + 1, maxHistory - 1));
        } else {
            setHistory(prev => {
                const newHistory = prev.slice(0, historyIndex + 1);
                newHistory.push(newEntry);
                if (newHistory.length > maxHistory) newHistory.shift();
                return newHistory;
            });
            setHistoryIndex(prev => Math.min(prev + 1, maxHistory - 1));
        }

        try {
            const results = await lookupYomitan(cleanText, prefixBytes, settings.resultGroupingMode || 'grouped', settings.yomitanLanguage || 'japanese');
            const loadedResults = results === 'loading' ? [] : ((results as any).terms || results || []);
            const loadedKanji = results === 'loading' ? [] : ((results as any).kanji || []);
            const isSystemLoading = results === 'loading';

            setHistory(prev => {
                const newHistory = [...prev];
                const idx = Math.min(historyIndex + 1, maxHistory - 1);
                if (newHistory[idx]) {
                    const matchedTerm = loadedResults[0]?.headword || loadedKanji[0]?.character || cleanText;
                    newHistory[idx] = {
                        ...newHistory[idx],
                        term: matchedTerm,
                        results: loadedResults,
                        kanjiResults: loadedKanji,
                        isLoading: false,
                        systemLoading: isSystemLoading
                    };
                }
                return newHistory;
            });
        } catch (err) {
            console.warn('Failed to lookup word', err);
            setHistory(prev => {
                const newHistory = [...prev];
                const idx = Math.min(historyIndex + 1, maxHistory - 1);
                if (newHistory[idx]) {
                    newHistory[idx] = { ...newHistory[idx], results: [], isLoading: false, systemLoading: false };
                }
                return newHistory;
            });
        }
    }, [navMode, historyIndex, maxHistory]);

    const handleKanjiClick = useCallback(async (char: string) => {
        const newEntry: LookupHistoryEntry = {
            term: char,
            results: [],
            kanjiResults: [],
            isLoading: true,
            systemLoading: false,
            isKanjiOnly: true
        };

        if (navMode === 'tabs') {
            setHistory(prev => {
                const newHistory = prev.slice(0, historyIndex + 1);
                newHistory.push(newEntry);
                if (newHistory.length > maxHistory) newHistory.shift();
                return newHistory;
            });
            setHistoryIndex(prev => Math.min(prev + 1, maxHistory - 1));
        } else {
            setHistory(prev => {
                const newHistory = prev.slice(0, historyIndex + 1);
                newHistory.push(newEntry);
                if (newHistory.length > maxHistory) newHistory.shift();
                return newHistory;
            });
            setHistoryIndex(prev => Math.min(prev + 1, maxHistory - 1));
        }

        try {
            const results = await lookupYomitan(char, 0, settings.resultGroupingMode || 'grouped', settings.yomitanLanguage || 'japanese');
            const loadedKanji = results === 'loading' ? [] : ((results as any).kanji || []);
            const isSystemLoading = results === 'loading';

            setHistory(prev => {
                const newHistory = [...prev];
                const idx = Math.min(historyIndex + 1, maxHistory - 1);
                if (newHistory[idx]) {
                    newHistory[idx] = {
                        ...newHistory[idx],
                        kanjiResults: loadedKanji,
                        isLoading: false,
                        systemLoading: isSystemLoading
                    };
                }
                return newHistory;
            });
        } catch (err) {
            console.warn('Failed to lookup kanji', err);
            setHistory(prev => {
                const newHistory = [...prev];
                const idx = Math.min(historyIndex + 1, maxHistory - 1);
                if (newHistory[idx]) {
                    newHistory[idx] = { ...newHistory[idx], kanjiResults: [], isLoading: false, systemLoading: false };
                }
                return newHistory;
            });
        }
    }, [navMode, historyIndex, maxHistory, settings.resultGroupingMode, settings.yomitanLanguage]);

    const navigateToHistory = useCallback((index: number) => {
        if (index >= 0 && index < history.length) {
            setHistoryIndex(index);
        }
    }, [history.length]);

    const goBack = useCallback(() => {
        if (historyIndex > 0) {
            setHistoryIndex(prev => prev - 1);
        }
    }, [historyIndex]);

    useLayoutEffect(() => {
        if (!dictPopup.visible) return;

        const visualViewport = window.visualViewport;
        const viewport = visualViewport
            ? {
                left: visualViewport.offsetLeft,
                top: visualViewport.offsetTop,
                right: visualViewport.offsetLeft + visualViewport.width,
                bottom: visualViewport.offsetTop + visualViewport.height,
            }
            : { left: 0, top: 0, right: window.innerWidth, bottom: window.innerHeight };

        const popupEl = popupRef.current;
        const viewportWidth = viewport.right - viewport.left;
        const viewportHeight = viewport.bottom - viewport.top;
        const maxWidthBase = Math.max(POPUP_MIN_WIDTH_PX, (viewportWidth - POPUP_GAP * 2) / popupScale);
        const baseWidth = Math.min(popupWidthPx, maxWidthBase);
        const maxHeightViewport = Math.max(120, (viewportHeight - POPUP_GAP * 2) / popupScale);
        const baseMaxHeight = Math.min(popupHeightPx, maxHeightViewport);
        const popupWidth = popupEl?.offsetWidth || baseWidth;
        const measuredHeight = popupEl?.offsetHeight || 0;
        const popupHeight = measuredHeight > 0 ? Math.min(measuredHeight, baseMaxHeight) : baseMaxHeight;
        const popupWidthScaled = popupWidth * popupScale;
        const popupHeightScaled = popupHeight * popupScale;

        const selectionRects = (() => {
            const selection = window.getSelection();
            if (!selection || selection.rangeCount === 0 || selection.isCollapsed) {
                return [];
            }
            const range = selection.getRangeAt(0);
            return Array.from(range.getClientRects())
                .map((rect) => ({ x: rect.left, y: rect.top, width: rect.width, height: rect.height }))
                .filter((rect) => rect.width > 0 && rect.height > 0);
        })();

        const sourceRects = dictPopup.highlight?.rects?.length
            ? dictPopup.highlight.rects
            : selectionRects;

        const fallbackRect = { x: dictPopup.x, y: dictPopup.y, width: 1, height: 1 };
        const rects = sourceRects.length ? sourceRects : [fallbackRect];

        let left = rects[0].x;
        let top = rects[0].y;
        let right = rects[0].x + rects[0].width;
        let bottom = rects[0].y + rects[0].height;
        for (let i = 1; i < rects.length; i += 1) {
            const rect = rects[i];
            left = Math.min(left, rect.x);
            top = Math.min(top, rect.y);
            right = Math.max(right, rect.x + rect.width);
            bottom = Math.max(bottom, rect.y + rect.height);
        }

        const rightSpace = viewport.right - right - POPUP_GAP;
        const leftSpace = left - viewport.left - POPUP_GAP;
        const aboveSpace = top - viewport.top - POPUP_GAP;
        const belowSpace = viewport.bottom - bottom - POPUP_GAP;

        const clamp = (value: number, min: number, max: number) => {
            if (max < min) return min;
            return Math.min(Math.max(value, min), max);
        };

        let finalLeft: number;
        let finalTop: number;

        if (rightSpace >= popupWidthScaled) {
            finalLeft = right + POPUP_GAP;
            finalTop = top;
        } else if (leftSpace >= popupWidthScaled) {
            finalLeft = left - POPUP_GAP - popupWidthScaled;
            finalTop = top;
        } else {
            const placeBelow = belowSpace >= popupHeightScaled || belowSpace >= aboveSpace;
            finalTop = placeBelow ? bottom + POPUP_GAP : top - POPUP_GAP - popupHeightScaled;
            finalLeft = left;
        }

        finalLeft = clamp(finalLeft, viewport.left + POPUP_GAP, viewport.right - popupWidthScaled - POPUP_GAP);
        finalTop = clamp(finalTop, viewport.top + POPUP_GAP, viewport.bottom - popupHeightScaled - POPUP_GAP);

        setPosStyle({ top: finalTop, left: finalLeft, maxHeight: `${baseMaxHeight}px`, width: `${baseWidth}px` });
    }, [
        dictPopup.visible,
        dictPopup.x,
        dictPopup.y,
        dictPopup.highlight,
        popupHeightPx,
        popupWidthPx,
        popupScale,
    ]);

    useLayoutEffect(() => {
        const el = backdropRef.current;
        if (!el || !dictPopup.visible) return;

        const closePopup = () => {
            window.getSelection()?.removeAllRanges();
            notifyPopupClosed();
            setDictPopup(prev => ({ ...prev, visible: false }));
        };
        const shouldIgnoreOpeningGesture = () => Date.now() - openedAtRef.current < 450;

        const onTouchStart = (e: TouchEvent) => {
            if (e.cancelable) e.preventDefault();
            e.stopPropagation();
        };

        const onTouchEnd = (e: TouchEvent) => {
            if (e.cancelable) e.preventDefault();
            e.stopPropagation();
            if (shouldIgnoreOpeningGesture()) return;
            closePopup();
        };

        const onClick = (e: MouseEvent) => {
            e.stopPropagation();
            if (shouldIgnoreOpeningGesture()) return;
            closePopup();
        };

        const onBlock = (e: Event) => e.stopPropagation();

        const opts = { passive: false };

        el.addEventListener('touchstart', onTouchStart, opts);
        el.addEventListener('touchend', onTouchEnd, opts);
        el.addEventListener('click', onClick, opts);
        el.addEventListener('mousedown', onBlock, opts);
        el.addEventListener('contextmenu', onClick, opts);

        return () => {
            el.removeEventListener('touchstart', onTouchStart, opts as any);
            el.removeEventListener('touchend', onTouchEnd, opts as any);
            el.removeEventListener('click', onClick, opts as any);
            el.removeEventListener('mousedown', onBlock, opts as any);
            el.removeEventListener('contextmenu', onClick, opts as any);
        };
    }, [dictPopup.visible, setDictPopup, notifyPopupClosed]);

    if (!dictPopup.visible) return null;

    const theme = getPopupTheme(settings.yomitanPopupTheme);

    const popupStyle: React.CSSProperties = {
        position: 'fixed',
        zIndex: 2147483647,
        width: popupWidthStyle,
        maxWidth: `calc((100% - ${POPUP_GAP * 2}px) / ${popupScale})`,
        overflowY: 'auto',
        backgroundColor: theme.bg, color: theme.fg, border: `1px solid ${theme.border}`,
        borderRadius: '8px', boxShadow: '0 8px 24px rgba(0,0,0,0.5)',
        padding: '16px', fontFamily: 'sans-serif', fontSize: '14px', lineHeight: '1.5',
        overflowWrap: 'break-word',
        wordBreak: 'normal',
        WebkitOverflowScrolling: 'touch',
        transform: `scale(${popupScale})`,
        transformOrigin: 'top left',
        ...posStyle
    };

    return createPortal(
        <>
            <style>{STRUCTURED_CONTENT_CSS}</style>
            {customPopupCss && <style>{customPopupCss}</style>}
            <HighlightOverlay />
            <div
                ref={backdropRef}
                className="yomitan-backdrop"
                style={{
                    position: 'fixed', top: 0, left: 0, right: 0, bottom: 0,
                    zIndex: 2147483646,
                    cursor: 'default',
                    outline: 'none',
                    backgroundColor: 'transparent',
                    touchAction: 'none',
                }}
            />

            <div
                ref={popupRef}
                tabIndex={0}
                className={`yomitan-popup ${settings.yomitanPopupTheme === 'dark' ? 'yomitan-popup-dark' : 'yomitan-popup-light'}`}
                style={{
                    ...popupStyle,
                    outline: 'none',
                }}
                onMouseDown={e => e.stopPropagation()}
                onTouchStart={e => e.stopPropagation()}
                onTouchEnd={e => e.stopPropagation()}
                onClick={e => e.stopPropagation()}
                onWheel={e => e.stopPropagation()}
            >
                {(navMode === 'tabs' ? history.length > 1 : historyIndex > 0) && (
                    <div className="lookup-history-nav" style={{ marginBottom: '8px' }}>
                        {navMode === 'tabs' ? (
                            <div style={{ display: 'flex', gap: '4px', overflowX: 'auto', paddingBottom: '4px', alignItems: 'center' }}>
                                {history.map((entry, i) => (
                                    <React.Fragment key={i}>
                                        {i > 0 && (
                                            <span style={{ color: theme.secondary, fontSize: '0.7em', flexShrink: 0 }}>
                                                →
                                            </span>
                                        )}
                                        <button
                                            onClick={() => navigateToHistory(i)}
                                            style={{
                                                padding: '4px 8px',
                                                borderRadius: '4px',
                                                border: 'none',
                                                background: i === historyIndex ? theme.accent : theme.hoverBg,
                                                color: i === historyIndex ? 'white' : theme.fg,
                                                cursor: 'pointer',
                                                fontSize: '0.75em',
                                                maxWidth: '100px',
                                                overflow: 'hidden',
                                                textOverflow: 'ellipsis',
                                                whiteSpace: 'nowrap',
                                                flexShrink: 0,
                                            }}
                                            title={entry.term}
                                        >
                                            {entry.term.slice(0, 12)}
                                            {entry.term.length > 12 ? '...' : ''}
                                        </button>
                                    </React.Fragment>
                                ))}
                            </div>
                        ) : (
                            historyIndex > 0 ? (
                                <button
                                    onClick={goBack}
                                    style={{
                                        padding: '4px 8px',
                                        borderRadius: '4px',
                                        border: 'none',
                                        background: theme.accent,
                                        color: 'white',
                                        cursor: 'pointer',
                                        fontSize: '0.75em',
                                    }}
                                >
                                    ← Back
                                </button>
                            ) : null
                        )}
                    </div>
                )}
                <DictionaryView
                    results={currentEntry?.isKanjiOnly ? [] : processedEntries}
                    isLoading={isLoading}
                    systemLoading={systemLoading}
                    onLinkClick={handleDefinitionLink}
                    onWordClick={handleWordClick}
                    onKanjiClick={handleKanjiClick}
                    context={dictPopup.context}
                    variant="popup"
                    popupTheme={theme}
                    kanjiResults={currentEntry?.isKanjiOnly || processedEntries.length === 0 || settings.yomitanShowKanjiInNormalLookup ? kanjiResults : []}
                    grouped={settings.resultGroupingMode === 'grouped'}
                />
            </div>
        </>,
        document.body
    );
};
