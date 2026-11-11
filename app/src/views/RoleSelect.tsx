/**
 * 'link' is two-way: this laptop shares its decks and shows the partner's.
 * 'sender' and 'viewer' are the two halves on their own (one-way).
 */
export type Role = 'link' | 'sender' | 'viewer';

/** Wizard step 1 (brief, Appendix A). The rest of the wizard is Milestone 5. */
export function RoleSelect({ onChoose }: { onChoose: (role: Role) => void }) {
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
      </div>
    </div>
  );
}
