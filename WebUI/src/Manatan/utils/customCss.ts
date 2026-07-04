export const buildScopedCustomCss = (rawCss: string | undefined, selector: string): string => {
    const css = rawCss?.trim();
    if (!css) {
        return '';
    }

    if (css.includes('{')) {
        return css;
    }

    return `${selector} {\n${css}\n}`;
};

const escapeHtmlAttribute = (value: string): string =>
    value
        .replace(/&/g, '&amp;')
        .replace(/"/g, '&quot;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');

const escapeCssString = (value: string): string =>
    value
        .replace(/\\/g, '\\\\')
        .replace(/"/g, '\\"');

export const buildDictionaryScopedCss = (
    dictionaryStyles: Record<string, string> | undefined,
    dictionaryNames?: string[],
): string => {
    if (!dictionaryStyles) {
        return '';
    }

    const allowedNames = dictionaryNames?.length ? new Set(dictionaryNames) : null;
    return Object.entries(dictionaryStyles)
        .flatMap(([dictionaryName, css]) => {
            const trimmedCss = css?.trim();
            if (!trimmedCss || (allowedNames && !allowedNames.has(dictionaryName))) {
                return [];
            }

            return `[data-dictionary="${escapeCssString(dictionaryName)}"] {\n${trimmedCss}\n}`;
        })
        .join('\n');
};

export type AnkiDefinitionHtmlOptions = {
    customCss?: string;
    dictionaryStyles?: Record<string, string>;
    dictionaryNames?: string[];
    wrapperClassName?: string;
    themeClassName?: string;
    wrapperSelector?: string;
};

export const buildAnkiDefinitionHtml = (
    contentHtml: string,
    {
        customCss,
        dictionaryStyles,
        dictionaryNames,
        wrapperClassName = 'yomitan-popup',
        themeClassName = '',
        wrapperSelector = '.anki-dictionary-view',
    }: AnkiDefinitionHtmlOptions = {},
): string => {
    const dictionaryCss = buildDictionaryScopedCss(dictionaryStyles, dictionaryNames);
    const scopedCustomCss = buildScopedCustomCss(customCss, wrapperSelector);
    const styleParts = [dictionaryCss, scopedCustomCss].filter(Boolean);
    const styleHtml = styleParts.length ? `<style>${styleParts.join('\n')}</style>` : '';
    const trimmedCustomCss = customCss?.trim() || '';
    const wrapperInlineCustomCss = trimmedCustomCss && !trimmedCustomCss.includes('{')
        ? trimmedCustomCss
        : '';
    const className = [
        'anki-dictionary-view',
        'dictionary-view',
        wrapperClassName,
        themeClassName,
    ]
        .filter(Boolean)
        .join(' ');
    const wrapperStyle = [
        'font-family: sans-serif',
        'font-size: 14px',
        'line-height: 1.5',
        'overflow-wrap: break-word',
        'word-break: normal',
        wrapperInlineCustomCss,
    ].filter(Boolean).join('; ');

    return `${styleHtml}<div class="${escapeHtmlAttribute(className)}" style="${escapeHtmlAttribute(wrapperStyle)}"><div class="entry"><div class="entry-body gloss-list definition-list">${contentHtml}</div></div></div>`;
};
