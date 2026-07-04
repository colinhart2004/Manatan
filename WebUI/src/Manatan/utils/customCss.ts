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

const stripCssComments = (css: string): string => css.replace(/\/\*[\s\S]*?\*\//g, '');

const findNextBrace = (css: string, start: number): number => {
    let quote: string | null = null;
    let escaped = false;
    for (let i = start; i < css.length; i += 1) {
        const char = css[i];
        if (quote) {
            if (escaped) {
                escaped = false;
            } else if (char === '\\') {
                escaped = true;
            } else if (char === quote) {
                quote = null;
            }
            continue;
        }
        if (char === '"' || char === "'") {
            quote = char;
            continue;
        }
        if (char === '{') {
            return i;
        }
    }
    return -1;
};

const findMatchingBrace = (css: string, openIndex: number): number => {
    let quote: string | null = null;
    let escaped = false;
    let depth = 0;
    for (let i = openIndex; i < css.length; i += 1) {
        const char = css[i];
        if (quote) {
            if (escaped) {
                escaped = false;
            } else if (char === '\\') {
                escaped = true;
            } else if (char === quote) {
                quote = null;
            }
            continue;
        }
        if (char === '"' || char === "'") {
            quote = char;
            continue;
        }
        if (char === '{') {
            depth += 1;
        } else if (char === '}') {
            depth -= 1;
            if (depth === 0) {
                return i;
            }
        }
    }
    return css.length - 1;
};

const findSelectorStart = (css: string): number => {
    let quote: string | null = null;
    let escaped = false;
    let parenDepth = 0;
    let bracketDepth = 0;
    let selectorStart = 0;
    for (let i = 0; i < css.length; i += 1) {
        const char = css[i];
        if (quote) {
            if (escaped) {
                escaped = false;
            } else if (char === '\\') {
                escaped = true;
            } else if (char === quote) {
                quote = null;
            }
            continue;
        }
        if (char === '"' || char === "'") {
            quote = char;
            continue;
        }
        if (char === '(') parenDepth += 1;
        else if (char === ')' && parenDepth > 0) parenDepth -= 1;
        else if (char === '[') bracketDepth += 1;
        else if (char === ']' && bracketDepth > 0) bracketDepth -= 1;
        else if (char === ';' && parenDepth === 0 && bracketDepth === 0) {
            selectorStart = i + 1;
        }
    }
    return selectorStart;
};

const splitSelectors = (selectorText: string): string[] => {
    const selectors: string[] = [];
    let quote: string | null = null;
    let escaped = false;
    let parenDepth = 0;
    let bracketDepth = 0;
    let current = '';
    for (const char of selectorText) {
        if (quote) {
            current += char;
            if (escaped) {
                escaped = false;
            } else if (char === '\\') {
                escaped = true;
            } else if (char === quote) {
                quote = null;
            }
            continue;
        }
        if (char === '"' || char === "'") {
            quote = char;
            current += char;
            continue;
        }
        if (char === '(') parenDepth += 1;
        else if (char === ')' && parenDepth > 0) parenDepth -= 1;
        else if (char === '[') bracketDepth += 1;
        else if (char === ']' && bracketDepth > 0) bracketDepth -= 1;
        if (char === ',' && parenDepth === 0 && bracketDepth === 0) {
            const trimmed = current.trim();
            if (trimmed) selectors.push(trimmed);
            current = '';
            continue;
        }
        current += char;
    }
    const trimmed = current.trim();
    if (trimmed) selectors.push(trimmed);
    return selectors;
};

const normalizeDeclarations = (declarations: string): string =>
    declarations
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean)
        .join(' ');

const combineSelectors = (parentSelectors: string[], selectorText: string): string[] => {
    const childSelectors = splitSelectors(selectorText);
    return parentSelectors.flatMap((parentSelector) =>
        childSelectors.map((childSelector) =>
            childSelector.includes('&')
                ? childSelector.replace(/&/g, parentSelector)
                : `${parentSelector} ${childSelector}`,
        ),
    );
};

const flattenCssBlock = (css: string, parentSelectors: string[]): string => {
    const declarations: string[] = [];
    const rules: string[] = [];
    let cursor = 0;

    const flushDeclarations = () => {
        const declarationText = normalizeDeclarations(declarations.join(''));
        declarations.length = 0;
        if (declarationText && parentSelectors.length) {
            rules.push(`${parentSelectors.join(', ')} { ${declarationText} }`);
        }
    };

    while (cursor < css.length) {
        const openIndex = findNextBrace(css, cursor);
        if (openIndex === -1) {
            declarations.push(css.slice(cursor));
            break;
        }

        const prefix = css.slice(cursor, openIndex);
        const selectorStart = findSelectorStart(prefix);
        declarations.push(prefix.slice(0, selectorStart));
        const selectorText = prefix.slice(selectorStart).trim();
        const closeIndex = findMatchingBrace(css, openIndex);
        const block = css.slice(openIndex + 1, closeIndex);

        if (!selectorText) {
            declarations.push(css.slice(openIndex, closeIndex + 1));
            cursor = closeIndex + 1;
            continue;
        }

        flushDeclarations();

        if (selectorText.startsWith('@')) {
            if (/^@(media|supports|container|layer)\b/.test(selectorText)) {
                const nestedRules = flattenCssBlock(block, parentSelectors);
                if (nestedRules) {
                    rules.push(`${selectorText} {\n${nestedRules}\n}`);
                }
            } else {
                rules.push(`${selectorText} { ${normalizeDeclarations(block)} }`);
            }
        } else {
            const childSelectors = combineSelectors(parentSelectors, selectorText);
            const childRules = flattenCssBlock(block, childSelectors);
            if (childRules) {
                rules.push(childRules);
            }
        }

        cursor = closeIndex + 1;
    }

    flushDeclarations();
    return rules.filter(Boolean).join('\n');
};

export const buildDictionaryScopedCss = (
    dictionaryStyles: Record<string, string> | undefined,
    dictionaryNames?: string[],
    baseSelector = '.yomitan-glossary',
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

            const dictionarySelector = `${baseSelector} [data-dictionary="${escapeCssString(dictionaryName)}"]`;
            const flattenedCss = flattenCssBlock(stripCssComments(trimmedCss), [dictionarySelector]);
            return flattenedCss ? [flattenedCss] : [];
        })
        .join('\n');
};

export type AnkiGlossaryDefinitionHtml = {
    dictionaryName: string;
    tags?: string[];
    contentHtml: string;
};

export type AnkiGlossaryHtmlOptions = {
    customCss?: string;
    dictionaryStyles?: Record<string, string>;
    wrapperClassName?: string;
};

export const buildAnkiGlossaryHtml = (
    definitions: AnkiGlossaryDefinitionHtml[],
    {
        customCss,
        dictionaryStyles,
        wrapperClassName = 'yomitan-glossary',
    }: AnkiGlossaryHtmlOptions = {},
): string => {
    if (!definitions.length) {
        return '';
    }

    const dictionariesWithStyle = new Set<string>();
    const listItems = definitions.map((definition) => {
        const tags = definition.tags?.filter(Boolean) ?? [];
        const headerText = tags.length
            ? `${tags.join(', ')}, ${definition.dictionaryName}`
            : definition.dictionaryName;
        const styleHtml = dictionariesWithStyle.has(definition.dictionaryName)
            ? ''
            : buildDictionaryScopedCss(dictionaryStyles, [definition.dictionaryName]);
        dictionariesWithStyle.add(definition.dictionaryName);

        return `<li data-dictionary="${escapeHtmlAttribute(definition.dictionaryName)}"><i>(${escapeHtmlAttribute(headerText)})</i><span>${definition.contentHtml}</span>${styleHtml ? `<style>${styleHtml}</style>` : ''}</li>`;
    }).join('');

    const scopedCustomCss = buildScopedCustomCss(customCss, '.yomitan-glossary');
    const customStyleHtml = scopedCustomCss ? `<style>${scopedCustomCss}</style>` : '';
    return `<div style="text-align: left;" class="${escapeHtmlAttribute(wrapperClassName)}"><ol>${listItems}</ol>${customStyleHtml}</div>`;
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
    const dictionaryCss = buildDictionaryScopedCss(dictionaryStyles, dictionaryNames, wrapperSelector);
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
