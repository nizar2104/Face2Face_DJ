import type { KeyInfo } from '../protocol/messages.ts';
import type { KeyNotation } from '../settings.ts';

/** The key in the chosen notation, with the other notation small beside it. */
export function KeyBadge({ keyInfo, notation }: { keyInfo: KeyInfo | null; notation: KeyNotation }) {
  if (keyInfo === null) return <span className="key key-missing" title="Key missing: analyse the track in your DJ software">–</span>;
  const main = notation === 'camelot' ? keyInfo.camelot : keyInfo.musical;
  const other = notation === 'camelot' ? keyInfo.musical : keyInfo.camelot;
  return (
    <span className="key">
      <span className="key-main">{main}</span>
      <span className="key-other">{other}</span>
    </span>
  );
}
