import { useState } from 'react';
import { DEFAULT_PORT } from './protocol/messages.ts';
import { RoleSelect } from './views/RoleSelect.tsx';
import type { Role } from './views/RoleSelect.tsx';
import { Sender } from './views/Sender.tsx';
import { Viewer } from './views/Viewer.tsx';

// Development overrides, e.g. http://localhost:1420/?role=viewer&sender=127.0.0.1:47811
const params = new URLSearchParams(window.location.search);

function initialRole(): Role | null {
  const role = params.get('role');
  return role === 'link' || role === 'sender' || role === 'viewer' ? role : null;
}

function senderUrl(): string {
  const sender = params.get('sender');
  return `ws://${sender !== null && /^[\w.\-[\]:]+$/.test(sender) ? sender : `127.0.0.1:${DEFAULT_PORT}`}`;
}

export function App() {
  // The role is asked on every launch in Milestone 0 (see docs/DECISIONS.md, D8).
  const [role, setRole] = useState<Role | null>(initialRole);
  if (role === null) return <RoleSelect onChoose={setRole} />;
  if (role === 'viewer') return <Viewer senderUrl={senderUrl()} onSwitchRole={() => setRole(null)} />;
  return <Sender link={role === 'link'} onSwitchRole={() => setRole(null)} />;
}
