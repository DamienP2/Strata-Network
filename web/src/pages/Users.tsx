import { useCallback, useEffect, useMemo, useState, type FormEvent } from 'react';
import { Ban, KeyRound, Pencil, Plus, Search, Trash2, Users as UsersIcon, X } from 'lucide-react';
import { t, tn } from '../i18n';
import { api, ROLES, useApp, useChanged, type PublicUser, type Role } from '../store';
import { Field, FieldGroup } from '../settings/ui';
import { Check } from '../ui/fields';
import { confirmDialog, promptDialog } from '../ui/dialogs';
import { errorText } from '../ui/serverError';
import { initial } from '../ui/UserMenu';
import { ROLE_ICON, roleHint, roleLabel, rolePlural } from '../ui/roles';
import { PageHead } from './SidePage';

type Filter = 'all' | Role | 'disabled';

export function Users() {
  const me = useApp((s) => s.auth?.user);
  const toast = useApp((s) => s.toast);
  const [users, setUsers] = useState<PublicUser[] | null>(null);
  const [q, setQ] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const [editing, setEditing] = useState<PublicUser | 'new' | null>(null);

  const load = useCallback(async () => {
    try {
      setUsers(await api<PublicUser[]>('GET', '/api/users'));
    } catch (e) {
      toast(errorText(e), 'error', (e as Error).message);
    }
  }, [toast]);
  useEffect(() => void load(), [load]);
  useChanged('users', load);

  const list = useMemo(() => users ?? [], [users]);
  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return list
      .filter((u) => (filter === 'all' ? true : filter === 'disabled' ? u.disabled : u.role === filter))
      .filter((u) => !needle || u.username.toLowerCase().includes(needle) || u.displayName.toLowerCase().includes(needle))
      .sort((a, b) => (a.displayName || a.username).localeCompare(b.displayName || b.username));
  }, [list, q, filter]);
  const disabledCount = list.filter((u) => u.disabled).length;

  const setPassword = async (u: PublicUser) => {
    const password = await promptDialog({ title: t('Password of {name}', { name: u.displayName || u.username }), label: t('New password'), placeholder: t('At least 8 characters'), password: true, confirm: t('Save') });
    if (password === null) return;
    try {
      await api('PUT', `/api/users/${u.id}`, { username: u.username, displayName: u.displayName, role: u.role, disabled: u.disabled, roleFixed: u.roleFixed, password });
      toast(t('Password changed'));
    } catch (e) {
      toast(errorText(e), 'error', (e as Error).message);
    }
  };

  const remove = async (u: PublicUser) => {
    if (!(await confirmDialog({ title: t('Delete the user “{name}”?', { name: u.displayName || u.username }), confirm: t('Delete'), danger: true }))) return;
    try {
      await api('DELETE', `/api/users/${u.id}`);
      void load();
    } catch (e) {
      toast(errorText(e), 'error', (e as Error).message);
    }
  };

  if (!users) return <div className="loading">{t('Loading…')}</div>;
  return (
    <>
      <PageHead title={t('Users')} sub={tn(list.length, '{n} user', '{n} users')}>
        <button className="btn primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> {t('New user')}
        </button>
      </PageHead>
      <div className="media-toolbar">
        <label className="search">
          <Search size={14} />
          <input value={q} placeholder={t('Search users')} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
        </label>
        <div className="asr-chips">
          <button className={`asr-chip ${filter === 'all' ? 'on' : ''}`} onClick={() => setFilter('all')}>
            {t('All users')} <span className="asr-chip-n">{list.length}</span>
          </button>
          {ROLES.map((r) => (
            <button key={r} className={`asr-chip ${filter === r ? 'on' : ''}`} onClick={() => setFilter(r)}>
              {rolePlural(r)} <span className="asr-chip-n">{list.filter((u) => u.role === r).length}</span>
            </button>
          ))}
          {disabledCount > 0 && (
            <button className={`asr-chip ${filter === 'disabled' ? 'on' : ''}`} onClick={() => setFilter('disabled')}>
              {t('Disabled')} <span className="asr-chip-n">{disabledCount}</span>
            </button>
          )}
        </div>
      </div>
      <div className="user-grid">
        {shown.length === 0 && (
          <div className="empty big">
            <UsersIcon size={22} />
            <p>{t('No match')}</p>
          </div>
        )}
        {shown.map((u) => {
          const RoleIcon = ROLE_ICON[u.role];
          const self = u.id === me?.id;
          return (
            <div key={u.id} className={`user-card role-${u.role} ${u.disabled ? 'dim' : ''}`} onClick={() => setEditing(u)}>
              <div className="user-card-head">
                <div className="user-avatar">{initial(u)}</div>
                <div className="user-card-name">
                  <strong>{u.displayName || u.username}</strong>
                  <span className="muted">
                    @{u.username}
                    {self && ` · ${t('you')}`}
                    {u.sso && <span className="badge sso">{t('SSO')}</span>}
                  </span>
                </div>
                <span className={`badge role-${u.role}`}>
                  <RoleIcon size={11} /> {roleLabel(u.role)}
                </span>
              </div>
              {u.disabled && (
                <span className="user-disabled">
                  <Ban size={12} /> {t('Account disabled')}
                </span>
              )}
              <div className="user-card-actions" onClick={(e) => e.stopPropagation()}>
                <button className="btn small" onClick={() => setEditing(u)}>
                  <Pencil size={13} /> {t('Edit')}
                </button>
                {!u.sso && (
                  <button className="btn small" onClick={() => void setPassword(u)}>
                    <KeyRound size={13} /> {t('Password')}
                  </button>
                )}
                <div className="spacer" />
                {!self && (
                  <button className="icon-btn ghost" title={t('Delete')} onClick={() => void remove(u)}>
                    <Trash2 size={14} />
                  </button>
                )}
              </div>
            </div>
          );
        })}
        <button className="project-new user-new" onClick={() => setEditing('new')}>
          <Plus size={22} />
          {t('New user')}
        </button>
      </div>
      {editing && (
        <UserModal
          user={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void load();
          }}
        />
      )}
    </>
  );
}

function UserModal({ user, onClose, onSaved }: { user: PublicUser | null; onClose: () => void; onSaved: () => void }) {
  const ssoActive = !!useApp((s) => s.auth?.sso);
  const toast = useApp((s) => s.toast);
  const [username, setUsername] = useState(user?.username ?? '');
  const [displayName, setDisplayName] = useState(user?.displayName ?? '');
  const [password, setPassword] = useState('');
  const [role, setRole] = useState<Role>(user?.role ?? 'viewer');
  const [disabled, setDisabled] = useState(user?.disabled ?? false);
  const [roleFixed, setRoleFixed] = useState(user?.roleFixed ?? false);
  const [busy, setBusy] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    if (password && password.length < 8) {
      toast(t('The password needs at least 8 characters'), 'error');
      return;
    }
    setBusy(true);
    const body = { username: username.trim(), displayName: displayName.trim(), role, disabled, password, roleFixed };
    try {
      if (user) await api('PUT', `/api/users/${user.id}`, body);
      else await api('POST', '/api/users', body);
      onSaved();
    } catch (err) {
      toast(errorText(err), 'error', (err as Error).message);
      setBusy(false);
    }
  };

  const canSubmit = !!username.trim() && (user !== null || ssoActive || password.length > 0);
  return (
    <div className="modal-back" onPointerDown={onClose}>
      <form className="modal wide" role="dialog" aria-labelledby="user-title" onPointerDown={(e) => e.stopPropagation()} onSubmit={submit}>
        <header>
          <h2 id="user-title">{user ? t('Edit {name}', { name: user.displayName || user.username }) : t('New user')}</h2>
          <button type="button" className="icon-btn ghost" title={t('Close')} onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="set-grid two">
          <Field label={t('User name')}>
            <input value={username} autoFocus={!user} autoComplete="off" onChange={(e) => setUsername(e.target.value)} />
          </Field>
          <Field label={t('Display name')}>
            <input value={displayName} placeholder={t('Optional')} autoComplete="off" onChange={(e) => setDisplayName(e.target.value)} />
          </Field>
          {user?.sso ? (
            <Field label={t('Sign-in')}>
              <input value={t('SSO')} readOnly />
            </Field>
          ) : (
            <Field label={user ? t('New password') : t('Password')}>
              <input type="password" value={password} autoComplete="new-password" placeholder={user ? t('Leave empty to keep it') : t('At least 8 characters')} onChange={(e) => setPassword(e.target.value)} />
            </Field>
          )}
          <FieldGroup label={t('State')}>
            <Check checked={disabled} onChange={setDisabled}>
              {t('Account disabled')}
            </Check>
          </FieldGroup>
        </div>
        <FieldGroup label={t('Role')}>
          <div className="role-pick">
            {ROLES.map((r) => {
              const Icon = ROLE_ICON[r];
              return (
                <button key={r} type="button" className={role === r ? 'on' : ''} title={roleHint(r)} aria-pressed={role === r} onClick={() => setRole(r)}>
                  <span className={`badge role-${r}`}>
                    <Icon size={11} /> {roleLabel(r)}
                  </span>
                </button>
              );
            })}
          </div>
        </FieldGroup>
        {ssoActive && (
          <Check checked={roleFixed} onChange={setRoleFixed}>
            {t('Role set by hand')}
          </Check>
        )}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            {t('Cancel')}
          </button>
          <button type="submit" className="btn primary" disabled={!canSubmit || busy}>
            {user ? <KeyRound size={14} /> : <Plus size={14} />} {user ? t('Save') : t('Create the user')}
          </button>
        </footer>
      </form>
    </div>
  );
}
