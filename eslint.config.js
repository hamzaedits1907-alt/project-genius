import js from '@eslint/js';
import globals from 'globals';
import reactHooks from 'eslint-plugin-react-hooks';
import tseslint from 'typescript-eslint';

const EFFECT = 'CallExpression[callee.name=/^use(Layout|Insertion)?Effect$/]';
const EFFECT_MEMBER = 'CallExpression[callee.property.name=/^use(Layout|Insertion)?Effect$/]';
export const EFFECT_RULES = [EFFECT, EFFECT_MEMBER].flatMap((call) => [
  { selector: `${call} > ArrowFunctionExpression.arguments:first-child[body.type!='BlockStatement']`, message: 'Effect callbacks must use a block body: an expression body returns its value, which React would call as the cleanup ("destroy is not a function").' },
  { selector: `${call} > :function.arguments:first-child[async=true]`, message: 'Effect callbacks must not be async: the returned Promise would be called as the cleanup. Call an async function inside the effect instead.' },
  { selector: `${call}[arguments.0.type='Identifier']`, message: 'Pass an inline block-bodied callback to effects, not a function reference, so its return value is visibly a cleanup or nothing.' },
]);

export default tseslint.config(
  { ignores: ['dist', 'target', 'src-tauri', 'crates', 'node_modules'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.{ts,tsx}'],
    languageOptions: { ecmaVersion: 2022, globals: globals.browser },
    plugins: { 'react-hooks': reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
      'no-empty': ['error', { allowEmptyCatch: false }],
      // React calls whatever an effect returns as its cleanup ("destroy is not a function" when
      // it is a Promise, null, or any other value). Effects must have a block body, must not be
      // async, and must not be a function reference whose return value we cannot see.
      'no-restricted-syntax': ['error', ...EFFECT_RULES],
    },
  },
);
