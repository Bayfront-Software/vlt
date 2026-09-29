// 手書きの線画アイコン（viewBox 0 0 20 20、stroke は currentColor）。
// 絵文字は環境で見た目が変わり安っぽく見えるので使わない。

const P = {
  login: '<circle cx="10" cy="7" r="3"/><path d="M4 17c0-3.3 2.7-5 6-5s6 1.7 6 5"/>',
  password: '<circle cx="7" cy="10" r="3.2"/><path d="M10.2 10H17M14.5 10v2.5M16.5 10v2"/>',
  api_credential: '<path d="M7 4c-2 0-2 1.5-2 3s-1 3-2 3c1 0 2 1.5 2 3s0 3 2 3M13 4c2 0 2 1.5 2 3s1 3 2 3c-1 0-2 1.5-2 3s0 3-2 3"/>',
  secure_note: '<path d="M5 3h7l3 3v11H5z"/><path d="M12 3v3h3M8 10h4M8 13h4"/>',
  credit_card: '<rect x="2.5" y="5" width="15" height="10.5" rx="1.8"/><path d="M2.5 8.5h15M5.5 12.5h3"/>',
  identity: '<rect x="2.5" y="4" width="15" height="12" rx="1.8"/><circle cx="7.5" cy="9" r="1.8"/><path d="M4.8 13.5c.4-1.4 1.4-2 2.7-2s2.3.6 2.7 2M12 8.5h3.5M12 11.5h3.5"/>',
  ssh_key: '<rect x="2.5" y="4" width="15" height="12" rx="1.8"/><path d="M5.5 8l2.2 2-2.2 2M9.5 12.5h4"/>',
  database: '<ellipse cx="10" cy="5" rx="6" ry="2.2"/><path d="M4 5v10c0 1.2 2.7 2.2 6 2.2s6-1 6-2.2V5M4 10c0 1.2 2.7 2.2 6 2.2s6-1 6-2.2"/>',
  server: '<rect x="3" y="3.5" width="14" height="5.5" rx="1.2"/><rect x="3" y="11" width="14" height="5.5" rx="1.2"/><path d="M6 6.2h.01M6 13.8h.01"/>',
  software_license: '<path d="M3 6.5a1.5 1.5 0 0 0 0-3h14a1.5 1.5 0 0 0 0 3v7a1.5 1.5 0 0 0 0 3H3a1.5 1.5 0 0 0 0-3z" transform="translate(0 0)"/><path d="M8 10h4"/>',
  wifi: '<path d="M2.5 8a11 11 0 0 1 15 0M5 10.8a7.2 7.2 0 0 1 10 0M7.6 13.5a3.4 3.4 0 0 1 4.8 0"/><circle cx="10" cy="16" r=".6"/>',
  bank_account: '<path d="M3 8l7-4.5L17 8M4.5 8.5v6M8 8.5v6M12 8.5v6M15.5 8.5v6M3 16.5h14"/>',
  document: '<path d="M5 2.5h6.5L15 6v11.5H5z"/><path d="M11.5 2.5V6H15"/>',
  environment: '<rect x="2.5" y="3.5" width="15" height="13" rx="1.8"/><path d="M5.5 7.5h2M5.5 10h2M5.5 12.5h2M9.5 7.5h5M9.5 10h5M9.5 12.5h3"/>',
  heart: '<path d="M10 16.5s-6.5-3.9-6.5-8.3A3.6 3.6 0 0 1 10 6.3a3.6 3.6 0 0 1 6.5 1.9c0 4.4-6.5 8.3-6.5 8.3z"/>',
  terminal: '<rect x="2.5" y="3.5" width="15" height="13" rx="1.8"/><path d="M5.5 8l2.5 2-2.5 2M9.5 12.5h4"/>',
  // UI
  all: '<rect x="3" y="3" width="5.5" height="5.5" rx="1"/><rect x="11.5" y="3" width="5.5" height="5.5" rx="1"/><rect x="3" y="11.5" width="5.5" height="5.5" rx="1"/><rect x="11.5" y="11.5" width="5.5" height="5.5" rx="1"/>',
  star: '<path d="M10 2.8l2.2 4.6 5 .6-3.7 3.4 1 5-4.5-2.5-4.5 2.5 1-5L2.8 8l5-.6z"/>',
  shield: '<path d="M10 2.5l6 2.2v4.8c0 3.8-2.6 6.4-6 8-3.4-1.6-6-4.2-6-8V4.7z"/><path d="M10 7v3.5M10 13h.01"/>',
  tag: '<path d="M3 3h6.5L17 10.5 10.5 17 3 9.5z"/><circle cx="6.5" cy="6.5" r="1"/>',
  trash: '<path d="M4 6h12M8 6V4h4v2M5.5 6l.8 11h7.4l.8-11"/>',
  eye: '<path d="M1.8 10S5 4.5 10 4.5 18.2 10 18.2 10 15 15.5 10 15.5 1.8 10 1.8 10z"/><circle cx="10" cy="10" r="2.5"/>',
  "eye-off": '<path d="M3 3l14 14M8.2 5a8.6 8.6 0 0 1 1.8-.2c5 0 8.2 5.2 8.2 5.2a14 14 0 0 1-2.4 3M12 14.9a7.7 7.7 0 0 1-2 .3C5 15.2 1.8 10 1.8 10a14 14 0 0 1 3.4-3.7"/>',
  copy: '<rect x="7" y="7" width="10" height="10" rx="1.5"/><path d="M13 7V4.5A1.5 1.5 0 0 0 11.5 3h-7A1.5 1.5 0 0 0 3 4.5v7A1.5 1.5 0 0 0 4.5 13H7"/>',
  more: '<circle cx="4.5" cy="10" r="1.1"/><circle cx="10" cy="10" r="1.1"/><circle cx="15.5" cy="10" r="1.1"/>',
  gear: '<circle cx="10" cy="10" r="2.6"/><path d="M10 2v2.2M10 15.8V18M2 10h2.2M15.8 10H18M4.3 4.3l1.6 1.6M14.1 14.1l1.6 1.6M4.3 15.7l1.6-1.6M14.1 5.9l1.6-1.6"/>',
  lock: '<rect x="4" y="9" width="12" height="8.5" rx="1.5"/><path d="M6.5 9V6.5a3.5 3.5 0 0 1 7 0V9"/>',
  dice: '<rect x="3" y="3" width="14" height="14" rx="3"/><path d="M7 7h.01M13 7h.01M10 10h.01M7 13h.01M13 13h.01" stroke-width="2.4"/>',
  plus: '<path d="M10 4v12M4 10h12"/>',
  x: '<path d="M5 5l10 10M15 5L5 15"/>',
  link: '<path d="M8.5 11.5a3 3 0 0 0 4.2 0l2.6-2.6a3 3 0 0 0-4.2-4.2l-1 1M11.5 8.5a3 3 0 0 0-4.2 0l-2.6 2.6a3 3 0 0 0 4.2 4.2l1-1"/>',
  download: '<path d="M10 3v10M6 9.5l4 4 4-4M4 17h12"/>',
  restore: '<path d="M4 10a6 6 0 1 0 1.8-4.3L4 7.5M4 3.5v4h4"/>',
  pencil: '<path d="M4 16l1-4 8.5-8.5 3 3L8 15z"/>',
  expand: '<path d="M3 8V3h5M17 12v5h-5M3 3l5 5M17 17l-5-5"/>',
  reference: '<path d="M7.5 5L3 10l4.5 5M12.5 5L17 10l-4.5 5"/>',
  refresh: '<path d="M16 10a6 6 0 1 1-1.8-4.3L16 7.5M16 3.5v4h-4"/>',
  folder: '<path d="M2.5 5.5a1.5 1.5 0 0 1 1.5-1.5h3.5l2 2H16a1.5 1.5 0 0 1 1.5 1.5v7.5A1.5 1.5 0 0 1 16 16.5H4A1.5 1.5 0 0 1 2.5 15z"/>',
  fingerprint: '<path d="M5.5 15.5c1-1.8 1.5-3.8 1.5-5.5a3 3 0 0 1 6 0c0 2.4-.4 4.6-1.3 6.6M10 10c0 3-.8 5.3-2 7.2M4 12.5c.4-1 .5-1.8.5-2.5a5.5 5.5 0 0 1 11 0c0 1.2-.1 2.3-.3 3.4M5.5 5.3A7.4 7.4 0 0 1 15.6 6"/>',
};

/** 種類ごとの色（明るさを揃えた落ち着いた色相。ダーク時は CSS で持ち上げる）。 */
export const TYPE_TINT = {
  login: "#3b6fd6",
  password: "#5b6b8c",
  api_credential: "#7a55c7",
  secure_note: "#b8860b",
  credit_card: "#2f8f5b",
  identity: "#1f8a8a",
  ssh_key: "#44546a",
  database: "#c46a1c",
  server: "#4b5bc4",
  software_license: "#b8487a",
  wifi: "#1b86b8",
  bank_account: "#2a7d64",
  document: "#8a6a4a",
  environment: "#5f7a1f",
};

export function icon(name, { size = 18, strokeWidth = 1.6 } = {}) {
  const body = P[name] ?? P.document;
  return `<svg class="ico" viewBox="0 0 20 20" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="${strokeWidth}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;
}

export function typeBadge(type, size = "sm") {
  const tint = TYPE_TINT[type] ?? TYPE_TINT.document;
  const px = size === "lg" ? 26 : 17;
  return `<span class="type-badge ${size}" style="--tint:${tint}">${icon(type, { size: px, strokeWidth: size === "lg" ? 1.5 : 1.7 })}</span>`;
}
