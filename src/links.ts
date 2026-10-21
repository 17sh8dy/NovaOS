/**
 * Outside links NovaOS can open. The key goes to Rust, which holds the allow-listed address, so
 * only these exact pages can ever be opened. The URLs here are shown to the user and used when
 * there is no desktop shell (browser preview).
 */
export const LINKS = {
  discord: { url: 'https://discord.gg/XBhER9Z6EB', short: 'discord.gg/XBhER9Z6EB' },
} as const;

export type LinkKey = keyof typeof LINKS;
