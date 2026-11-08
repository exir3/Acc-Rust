import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (relative) => readFileSync(path.join(projectRoot, relative), 'utf8');

const indexHtml = read('ui/index.html');
const menu = read('ui/js/core.js');
const pageMeta = read('ui/js/pages-core.js');
const filters = read('ui/js/filters.js');

const supplementaryStatements = [
  {
    page: 'equity',
    script: 'equity-changes.js',
    command: 'get_equity_changes_statement',
    preview: 'open_equity_changes_preview',
  },
  {
    page: 'cashflow',
    script: 'cash-flow.js',
    command: 'get_cash_flow_statement',
    preview: 'open_cash_flow_preview',
  },
  {
    page: 'cogs',
    script: 'cogs.js',
    command: 'get_cogs_statement',
    preview: 'open_cogs_preview',
  },
  {
    page: 'appropriation',
    script: 'appropriation.js',
    command: 'get_profit_appropriation_statement',
    preview: 'open_profit_appropriation_preview',
  },
];

for (const statement of supplementaryStatements) {
  test(`supplementary statement "${statement.page}" is wired into the UI`, () => {
    assert.match(
      indexHtml,
      new RegExp(`<script src="js/pages/${statement.script}"></script>`),
      `${statement.script} must be loaded by index.html`,
    );
    assert.match(
      menu,
      new RegExp(`page: '${statement.page}'`),
      `the reports menu must link to the ${statement.page} page`,
    );
    assert.match(
      pageMeta,
      new RegExp(`${statement.page}: \\[`),
      `pageMeta must define a title for ${statement.page}`,
    );
    assert.match(
      filters,
      new RegExp(`${statement.page}: \\{`),
      `defaultFilters must define filters for ${statement.page}`,
    );

    const page = read(`ui/js/pages/${statement.script}`);
    assert.match(page, new RegExp(`PAGES\\.${statement.page} = \\{`));
    assert.match(page, new RegExp(`invoke\\('${statement.command}'`));
    assert.match(page, new RegExp(`'${statement.preview}'`));
  });
}
