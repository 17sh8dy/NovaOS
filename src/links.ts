/**
 * Outside links NovaOS can open. The key goes to Rust, which holds the allow-listed address, so
 * only these exact pages can ever be opened. The URLs here are shown to the user and used when
 * there is no desktop shell (browser preview).
 */
export const LINKS = {
  discord: { url: 'https://discord.gg/XBhER9Z6EB', short: 'discord.gg/XBhER9Z6EB' },
  // Places to find cursor files (Nova Studio → Cursor).
  'rw-designer': {
    url: 'https://www.rw-designer.com/cursor-library',
    short: 'rw-designer.com/cursor-library',
  },
  'custom-cursor': { url: 'https://custom-cursor.com/', short: 'custom-cursor.com' },
  'cursors-4u': { url: 'https://www.cursors-4u.com/', short: 'cursors-4u.com' },
} as const;

export type LinkKey = keyof typeof LINKS;
