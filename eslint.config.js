import js from '@eslint/js';
import globals from 'globals';
import noUnsanitized from 'eslint-plugin-no-unsanitized';
import promise from 'eslint-plugin-promise';
import uiGlobals from './eslint.ui-globals.js';

/**
 * Static analysis for the webview UI.
 *
 * The UI is loaded as classic scripts that share one global scope. The
 * intentional top-level names are listed in `eslint.ui-globals.js`, so
 * `no-undef` still catches typos while `no-unused-vars` is limited to local
 * variables. Everything else is checked strictly, including promise handling
 * and DOM-injection sinks.
 */
export default [
  {
    ignores: [
      'node_modules/**',
      'src-tauri/target/**',
      'playwright-report/**',
      'test-results/**',
    ],
  },
  js.configs.recommended,
  promise.configs['flat/recommended'],
  noUnsanitized.configs.recommended,
  {
    files: ['ui/**/*.js'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'script',
      globals: {
        ...globals.browser,
        ...uiGlobals,
      },
    },
    rules: {
      'no-unused-vars': [
        'error',
        {
          vars: 'local',
          args: 'after-used',
          caughtErrors: 'none',
          ignoreRestSiblings: true,
          argsIgnorePattern: '^_',
        },
      ],
      'no-redeclare': ['error', { builtinGlobals: false }],
      'no-shadow': 'error',
      eqeqeq: ['error', 'always', { null: 'ignore' }],
      'no-var': 'error',
      // Globals are written by other classic scripts, which this per-file
      // analysis cannot see; prefer-const would produce false positives.
      'prefer-const': 'off',
      'no-use-before-define': ['error', { functions: false, classes: true, variables: false }],
      'no-eval': 'error',
      'no-implied-eval': 'error',
      'no-new-func': 'error',
      'no-script-url': 'error',
      'no-prototype-builtins': 'error',
      'no-throw-literal': 'error',
      'no-promise-executor-return': 'error',
      'no-async-promise-executor': 'error',
      'no-await-in-loop': 'error',
      // The UI state objects are mutated deliberately after awaits; this rule
      // reports every such assignment and produces only false positives here.
      'require-atomic-updates': 'off',
      'no-unmodified-loop-condition': 'error',
      'no-self-assign': 'error',
      'no-self-compare': 'error',
      'no-sequences': 'error',
      'no-constant-condition': 'error',
      'no-duplicate-case': 'error',
      'no-fallthrough': 'error',
      'no-unreachable': 'error',
      'no-empty': ['error', { allowEmptyCatch: true }],
      'no-extra-boolean-cast': 'error',
      'no-useless-escape': 'error',
      'no-control-regex': 'error',
      'no-misleading-character-class': 'error',
      'no-template-curly-in-string': 'error',
      'no-unused-private-class-members': 'error',
      'no-undef-init': 'error',
      'no-return-assign': 'error',
      'no-lone-blocks': 'error',
      // The project's HTML helpers: `esc()` and `escapeVoucherPickerHtml()`
      // escape values, while the render helpers return markup built only from
      // escaped values or static templates. Treating them as sanitizers keeps
      // the rule focused on genuinely unescaped sinks.
      'no-unsanitized/property': [
        'error',
        {
          escape: {
            methods: [
              'esc',
              'escapeVoucherPickerHtml',
              'icon',
              'voucherRowTemplate',
              'trialGridMarkup',
              'trialFilterPanelMarkup',
              'auditLogCard',
            ],
          },
          variableTracing: true,
        },
      ],
      'no-unsanitized/method': [
        'error',
        {
          escape: {
            methods: [
              'esc',
              'escapeVoucherPickerHtml',
              'icon',
              'voucherRowTemplate',
              'trialGridMarkup',
              'trialFilterPanelMarkup',
              'auditLogCard',
            ],
          },
          variableTracing: true,
        },
      ],
    },
  },
  {
    files: ['scripts/**/*.mjs', 'tests/**/*.mjs', 'playwright.config.mjs', 'eslint.config.js'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
      globals: {
        ...globals.node,
        // Playwright `page.evaluate` callbacks run in the browser page.
        ...globals.browser,
      },
    },
    rules: {
      'no-unused-vars': ['error', { args: 'after-used', caughtErrors: 'none' }],
      eqeqeq: ['error', 'always', { null: 'ignore' }],
      'no-var': 'error',
      'prefer-const': 'error',
      'no-eval': 'error',
      'no-implied-eval': 'error',
      'no-new-func': 'error',
      'no-throw-literal': 'error',
      'no-promise-executor-return': 'error',
      'no-async-promise-executor': 'error',
      // File scans and Playwright setup steps must run in order.
      'no-await-in-loop': 'off',
      'no-self-assign': 'error',
      'no-self-compare': 'error',
      'no-sequences': 'error',
      'no-constant-condition': 'error',
      'no-duplicate-case': 'error',
      'no-fallthrough': 'error',
      'no-unreachable': 'error',
      'no-empty': ['error', { allowEmptyCatch: true }],
      'no-useless-escape': 'error',
    },
  },
  {
    files: ['tests/**/*.js'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'script',
      globals: {
        ...globals.browser,
      },
    },
    rules: {
      'no-unused-vars': ['error', { args: 'after-used', caughtErrors: 'none' }],
      eqeqeq: ['error', 'always', { null: 'ignore' }],
      'no-var': 'error',
      'prefer-const': 'error',
      'no-eval': 'error',
      'no-implied-eval': 'error',
      'no-new-func': 'error',
      'no-throw-literal': 'error',
      'no-promise-executor-return': 'error',
      'no-async-promise-executor': 'error',
      'no-self-assign': 'error',
      'no-self-compare': 'error',
      'no-sequences': 'error',
      'no-constant-condition': 'error',
      'no-duplicate-case': 'error',
      'no-fallthrough': 'error',
      'no-unreachable': 'error',
      'no-empty': ['error', { allowEmptyCatch: true }],
      'no-useless-escape': 'error',
    },
  },
];
