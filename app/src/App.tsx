import { useState } from 'react';
import { DEFAULT_PORT } from './protocol/messages.ts';
import { isTauri, localSenderUrl, stopSender } from './tauri.ts';
import { RoleSelect } from './views/RoleSelect.tsx';
import type { Role } from './views/RoleSelect.tsx';
import { Sender } from './views/Sender.tsx';
import { Viewer } from './views/Viewer.tsx';

// Development overrides, e.g. http://localhost:1420/?role=viewer&sender=127.0.0.1:47811
const params = new URLSearchParams(window.location.search);

function initialRole(): Role | null {
  const role = params.get('role');
  return role === 'link' || role === 'sender' || role === 'viewer' || role === 'pair' ? role : null;
}

function senderUrl(): string {
  const sender = params.get('sender');
  return sender !== null && /^[\w.\-[\]:]+$/.test(sender) ? `ws://${sender}` : localSenderUrl(DEFAULT_PORT);
}

/**
 * Names for the "laptops" this page can be (see tauri.ts): one, or two on the
 * test page. Kept for the life of the tab, so a reload keeps its Senders.
 */
function laptopSessions(): readonly [string, string] {
  const key = 'face2face-dj.laptops';
  const make = (): string =>
    typeof crypto.randomUUID === 'function' ? crypto.randomUUID() : `laptop-${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
  try {
    const saved: unknown = JSON.parse(sessionStorage.getItem(key) ?? 'null');
    if (Array.isArray(saved) && typeof saved[0] === 'string' && typeof saved[1] === 'string') return [saved[0], saved[1]];
    const made = [make(), make()] as const;
    sessionStorage.setItem(key, JSON.stringify(made));
    return made;
  } catch {
    return [make(), make()];
  }
}

export function App() {
  // The role is asked on every launch in Milestone 0 (see docs/DECISIONS.md, D8).
  const [role, setRole] = useState<Role | null>(initialRole);
  // One "laptop" per page; the two-laptop test page has a second one.
  const [sessions] = useState(laptopSessions);
  const back = (): void => setRole(null);

  if (role === null) return <RoleSelect browser={!isTauri()} onChoose={setRole} />;
  if (role === 'viewer') return <Viewer senderUrl={senderUrl()} onSwitchRole={back} />;
  if (role === 'pair') {
    // Browser test page: both laptops of a linked pair, one above the other.
    const leave = (): void => {
      for (const session of sessions) stopSender(session).catch(() => undefined);
      back();
    };
    return (
      <div className="pair">
        {sessions.map((session) => (
          <Sender key={session} link session={session} onSwitchRole={leave} />
        ))}
      </div>
    );
  }
  return <Sender link={role === 'link'} session={sessions[0]} onSwitchRole={back} />;
}
