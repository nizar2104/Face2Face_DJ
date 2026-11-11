export type Role = 'sender' | 'viewer';

/** Wizard step 1 (brief, Appendix A). The rest of the wizard is Milestone 5. */
export function RoleSelect({ onChoose }: { onChoose: (role: Role) => void }) {
  return (
    <div className="role-select">
      <h1>Face2Face DJ</h1>
      <p>
        On the computer running Rekordbox or Traktor, choose <b>Share my decks</b>. On the other computer, choose{' '}
        <b>View partner</b>.
      </p>
      <div className="role-buttons">
        <button onClick={() => onChoose('sender')}>
          <span className="role-name">Share my decks</span>
          <span className="role-hint">Sender · computer 1</span>
        </button>
        <button onClick={() => onChoose('viewer')}>
          <span className="role-name">View partner</span>
          <span className="role-hint">Viewer · computer 2</span>
        </button>
      </div>
    </div>
  );
}
