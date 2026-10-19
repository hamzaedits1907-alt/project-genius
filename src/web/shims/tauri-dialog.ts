// Browser replacement for '@tauri-apps/plugin-dialog': native file dialogs do not exist in a browser.
// File attachments use <input type="file"> in the interface instead.
const unsupported = () => Promise.reject({ kind: 'unsupported', message: 'Choosing folders and files on your computer is only available in Genius Desktop.' });

export const open = unsupported;
export const save = unsupported;
export const message = unsupported;
export const ask = unsupported;
export const confirm = unsupported;
