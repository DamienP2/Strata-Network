import { t } from '../i18n';

/** Motif anglais du serveur → texte de l'interface. */
const KNOWN: [RegExp, () => string][] = [
  [/failed to fetch|networkerror|load failed|network request failed/i, () => t('The server is not responding')],
  [/^wrong user name or password/i, () => t('Wrong user name or password.')],
  [/^too many tries/i, () => t('Too many tries: wait a moment, then try again.')],
  [/^login required/i, () => t('Login required')],
  [/^you do not have the rights/i, () => t('You do not have the rights for this')],
  [/^no such user/i, () => t('No such user')],
  [/^already set up/i, () => t('The server is already set up')],
  [/^request sent by another site refused/i, () => t('Request sent by another site refused')],
  [/^you cannot delete your own account/i, () => t('You cannot delete your own account')],
  [/at least one active admin/i, () => t('At least one active administrator is needed')],
  [/first account must be an active admin/i, () => t('The first account must be an active administrator')],
  [/password needs at least 8/i, () => t('The password needs at least 8 characters')],
  [/password is too long/i, () => t('The password is too long')],
  [/user name is already taken/i, () => t('This user name is already taken')],
  [/user name cannot be empty/i, () => t('The user name cannot be empty')],
  [/user name is too long/i, () => t('The user name is too long (64 characters at most)')],
  [/user name only has letters/i, () => t('A user name only has letters, digits and . - _ @')],
  [/signs in with the directory/i, () => t('This account signs in with the directory (SSO)')],
  [/unknown theme/i, () => t('Unknown theme')],
  [/server name is too long/i, () => t('The server name is too long (64 characters at most)')],
  [/https port must differ/i, () => t('The HTTPS port must differ from the HTTP port')],
  [/sso issuer is missing/i, () => t('The SSO issuer is missing')],
  [/sso client id is missing/i, () => t('The SSO client ID is missing')],
  [/groups claim cannot be empty/i, () => t('The groups claim cannot be empty')],
  [/server name cannot be empty/i, () => t('The server name cannot be empty')],
  [/repository is written owner\/name/i, () => t('The repository is written owner/name')],
  [/github refused the token/i, () => t('GitHub refused the token (expired or revoked?)')],
  [/github refused the request/i, () => t('GitHub refused the request (rate limit, or the token cannot read the repository)')],
  [/not found: the repository is private/i, () => t('Repository not found: it is private, add a read-only token')],
  [/unknown language/i, () => t('Unknown language')],
  [/a scan is already running/i, () => t('A scan is already running')],
  [/a check is already running/i, () => t('A check is already running')],
  [/this vlan exists/i, () => t('This VLAN already exists')],
  [/a vlan is a number from 1 to 4094/i, () => t('A VLAN is a number from 1 to 4094')],
  [/not an ipv4 address/i, () => t('This is not an IPv4 address')],
  [/the file has no ip column/i, () => t('The file has no ip column')],
  [/the file is empty/i, () => t('The file is empty')],
  [/the name cannot be empty/i, () => t('The name cannot be empty')],
  [/the range ends before it starts/i, () => t('The range ends before it starts')],
  [/the prefix is a number from 0 to 32/i, () => t('The prefix is a number from 0 to 32')],
  [/a switch has at least one port/i, () => t('A switch has at least one port')],
  [/the colour is written #rrggbb/i, () => t('The color is written #rrggbb')],
  [/the port of this stream is not known/i, () => t('The port of this stream is not known')],
  [/no network interface/i, () => t('No network interface')],
  [/^no such network/i, () => t('No such network')],
  [/^no such switch/i, () => t('No such switch')],
  [/^no such vlan/i, () => t('No such VLAN')],
  [/^no such snmp agent/i, () => t('No such SNMP agent')],
  [/^no answer/i, () => t('No answer')],
];

/** Le message d'erreur à montrer ; le texte brut reste en infobulle. */
export function friendlyError(raw: string): string {
  for (const [re, text] of KNOWN) if (re.test(raw)) return text();
  return raw;
}

/** Le message d'une exception, traduit s'il est connu. */
export function errorText(e: unknown): string {
  return friendlyError(e instanceof Error ? e.message : String(e));
}
