import { describe, expect, test } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const layoutCss = readFileSync(join(process.cwd(), 'src/styles/layout.css'), 'utf8');
const panelsCss = readFileSync(join(process.cwd(), 'src/styles/panels.css'), 'utf8');

describe('mobile form controls', () => {
  test('keeps mobile inputs at 16px or larger so iOS does not zoom on focus', () => {
    expect(layoutCss).toMatch(/@media \(max-width: 700px\)[\s\S]*font-size:\s*16px/);
    expect(panelsCss).toMatch(/@media \(max-width: 700px\)[\s\S]*\.mcp-json-textarea[\s\S]*font-size:\s*16px/);
  });
});

describe('section tab highlights', () => {
  test('keeps active detail section borders square', () => {
    expect(panelsCss).toMatch(/\.detail-section-chip\s*\{[\s\S]*border-radius:\s*0;/);
  });
});

describe('activity controls', () => {
  test('use the app control radius rather than pill styling', () => {
    const start = panelsCss.indexOf('/* LLM tool-call activity. */');
    const end = panelsCss.indexOf('/* Detail page metadata + body view. */');
    const activityCss = panelsCss.slice(start, end);

    expect(activityCss).toContain('border-radius: var(--radius)');
    expect(activityCss).not.toMatch(/border-radius:\s*999px/);
  });
});
