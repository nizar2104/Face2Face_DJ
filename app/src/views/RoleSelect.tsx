/**
 * 'link' is two-way: this laptop shares its decks and shows the partner's.
 * 'sender' and 'viewer' are the two halves on their own (one-way).
 * 'pair' is a browser test page: both laptops of a linked pair in one window.
 */
export type Role = 'link' | 'sender' | 'viewer' | 'pair';

interface Props {
  /** Running in a plain browser (development), not in the desktop app. */
  browser: boolean;
  onChoose: (role: Role) => void;
}

/** Wizard step 1 (brief, Appendix A). The rest of the wizard is Milestone 5. */
export function RoleSelect({ browser, onChoose }: Props) {
  return (
    <div className="role-select">
      <img className="brand-logo" src="/nomad-logo.png" alt="NO-MAD" />
      <h1>Face2Face DJ</h1>
      <p>
        Choose <b>Link up</b> on both laptops: each one shares its decks and shows the other's.
      </p>
      <div className="role-buttons">
        <button onClick={() => onChoose('link')}>
          <span className="role-name">Link up</span>
          <span className="role-hint">Share my decks and see my partner's</span>
        </button>
      </div>
      <div className="role-more">
        One-way:
        <button onClick={() => onChoose('sender')}>Share only</button>
        <button onClick={() => onChoose('viewer')}>View only</button>
        {browser && (
          <>
            <span>Test:</span>
            <button onClick={() => onChoose('pair')}>Both laptops here</button>
          </>
        )}
      </div>
    </div>
  );
}
