import { useEffect, useRef } from 'react';
import { EditorState, Compartment, type Extension } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection, highlightSpecialChars } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { syntaxHighlighting, indentOnInput, bracketMatching, foldGutter, foldKeymap } from '@codemirror/language';
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search';
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';
import { javascript } from '@codemirror/lang-javascript';
import { css } from '@codemirror/lang-css';
import { html } from '@codemirror/lang-html';
import { json } from '@codemirror/lang-json';
import { markdown } from '@codemirror/lang-markdown';
import { getChunks, unifiedMergeView } from '@codemirror/merge';
import { classHighlighter } from '@lezer/highlight';
import { languageFor } from '../lib/logic';
import './syntax.css';

function langExt(path: string): Extension {
  switch (languageFor(path)) {
    case 'ts':
      return javascript({ typescript: true });
    case 'tsx':
      return javascript({ typescript: true, jsx: true });
    case 'js':
      return javascript();
    case 'jsx':
      return javascript({ jsx: true });
    case 'css':
      return css();
    case 'html':
      return html();
    case 'json':
      return json();
    case 'md':
      return markdown();
    default:
      return [];
  }
}

interface Props {
  path: string;
  value: string;
  onChange?: (v: string) => void;
  onSave?: () => void;
  readOnly?: boolean;
  /** Show an inline diff against this original text. */
  original?: string | null;
  /** Keep the view scrolled to the end (live streaming). */
  follow?: boolean;
}

export default function CodeEditor({ path, value, onChange, onSave, readOnly, original, follow }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const cbs = useRef({ onChange, onSave });
  cbs.current = { onChange, onSave };
  const ro = useRef(new Compartment());

  useEffect(() => {
    if (!host.current) return;
    const exts: Extension[] = [
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightSpecialChars(),
      history(),
      foldGutter(),
      drawSelection(),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      syntaxHighlighting(classHighlighter),
      langExt(path),
      EditorState.tabSize.of(2),
      ro.current.of([EditorState.readOnly.of(!!readOnly), EditorView.editable.of(!readOnly)]),
      keymap.of([
        { key: 'Mod-s', preventDefault: true, run: () => (cbs.current.onSave?.(), true) },
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...searchKeymap,
        ...historyKeymap,
        ...foldKeymap,
        indentWithTab,
      ]),
      EditorView.updateListener.of((u) => {
        if (u.docChanged && u.transactions.some((t) => t.isUserEvent('input') || t.isUserEvent('delete') || t.isUserEvent('undo') || t.isUserEvent('redo') || t.isUserEvent('move'))) {
          cbs.current.onChange?.(u.state.doc.toString());
        }
      }),
    ];
    if (original != null) exts.push(unifiedMergeView({ original, mergeControls: false, highlightChanges: true, gutter: true }));
    const v = new EditorView({ state: EditorState.create({ doc: value, extensions: exts }), parent: host.current });
    view.current = v;
    if (original != null) {
      // Bring the first changed region into view so reviewers see the change immediately.
      const first = getChunks(v.state)?.chunks[0];
      if (first) v.dispatch({ effects: EditorView.scrollIntoView(Math.min(first.fromB, v.state.doc.length), { y: 'center' }) });
    }
    return () => {
      v.destroy();
      view.current = null;
    };
    // Recreate only when the file identity or diff base changes; value updates are dispatched below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, original]);

  useEffect(() => {
    const v = view.current;
    if (!v) return;
    const cur = v.state.doc.toString();
    if (cur === value) return;
    // Streaming appends are applied as appends to keep the viewport stable.
    const changes = value.startsWith(cur) ? { from: cur.length, insert: value.slice(cur.length) } : { from: 0, to: cur.length, insert: value };
    v.dispatch({ changes });
    if (follow) v.dispatch({ effects: EditorView.scrollIntoView(v.state.doc.length, { y: 'end' }) });
  }, [value, follow]);

  useEffect(() => {
    view.current?.dispatch({ effects: ro.current.reconfigure([EditorState.readOnly.of(!!readOnly), EditorView.editable.of(!readOnly)]) });
  }, [readOnly]);

  return <div className="editor-host" ref={host} data-testid="code-editor" />;
}
