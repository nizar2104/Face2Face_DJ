import { useState } from 'react';
import { formatCode, parseAddress, parseCode } from '../pairing.ts';
import type { Partner } from '../pairing.ts';
import type { LocalAddress } from '../tauri.ts';

interface Props {
  /**
   * What the partner needs from this laptop: its addresses (null while they
   * are being looked up) and its pairing code. Absent when this laptop only
   * views, and so has nothing for a partner to connect to.
   */
  mine?: { addresses: LocalAddress[] | null; code: string };
  /** The partner entered before, to start from. */
  current: Partner | null;
  /** What the "this computer" shortcut says, e.g. "My partner is on this computer". */
  localLabel: string;
  onChoose: (partner: Partner) => void;
  /** Back to the screen behind the panel, when a partner is already set. */
  onCancel?: () => void;
  /** Back to the start screen, for a role chosen by mistake. */
  onHome: () => void;
}

/**
 * Asks for the partner's address and pairing code, and shows this laptop's
 * own for the partner to type. Each DJ does this once; it is remembered.
 */
export function PartnerSetup({ mine, current, localLabel, onChoose, onCancel, onHome }: Props) {
  const [address, setAddress] = useState(current?.kind === 'network' ? current.address : '');
  const [code, setCode] = useState(current?.kind === 'network' ? formatCode(current.code) : '');
  const [problem, setProblem] = useState<string | null>(null);

  const submit = (event: React.FormEvent): void => {
    event.preventDefault();
    const partnerAddress = parseAddress(address);
    const partnerCode = parseCode(code);
    if (partnerAddress === null) setProblem("Enter the address shown on your partner's laptop, like 192.168.1.80.");
    else if (partnerCode === null) setProblem("Enter the 6-digit code shown on your partner's laptop.");
    else onChoose({ kind: 'network', address: partnerAddress, code: partnerCode });
  };

  let own = null;
  if (mine !== undefined) {
    const addresses = mine.addresses;
    own = (
      <p className="setup-mine">
        {addresses === null && 'Looking up this laptop’s address…'}
        {addresses !== null && addresses.length === 0 && (
          <span className="setup-problem">This laptop is not on a network. Join your partner’s Wi-Fi, or connect a cable.</span>
        )}
        {addresses !== null && addresses.length > 0 && (
          <>
            This laptop:{' '}
            {addresses.map((a, i) => (
              <span key={a.address}>
                {i > 0 && ' or '}
                <b title={a.interface}>{a.address}</b>
              </span>
            ))}
            {' · code '}
            <b>{formatCode(mine.code)}</b>
          </>
        )}
      </p>
    );
  }

  return (
    <form className="setup" onSubmit={submit}>
      <button type="button" className="setup-home" title="Back to the start screen" onClick={onHome}>‹ Home</button>
      <p className="notice-title">{mine !== undefined ? 'Link to your partner' : 'Connect to a Sender'}</p>
      {own}
      <div className="setup-row">
        <input
          aria-label="Partner's address"
          placeholder="Partner's address"
          value={address}
          autoFocus
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          onChange={(e) => { setAddress(e.target.value); setProblem(null); }}
        />
        <input
          className="setup-code"
          aria-label="Partner's code"
          placeholder="Code"
          inputMode="numeric"
          value={code}
          onChange={(e) => { setCode(e.target.value); setProblem(null); }}
        />
        <button className="text-button" type="submit">Link</button>
      </div>
      <p className={problem !== null ? 'setup-problem' : 'notice-detail'}>
        {problem ?? 'Both laptops on the same network: the same Wi-Fi, a phone hotspot, or a cable between them.'}
      </p>
      <div className="setup-more">
        <button type="button" onClick={() => onChoose({ kind: 'local' })}>{localLabel}</button>
        {onCancel && <button type="button" onClick={onCancel}>Cancel</button>}
      </div>
    </form>
  );
}
