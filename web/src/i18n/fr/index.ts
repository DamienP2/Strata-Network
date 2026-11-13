import { common } from './common';
import { shell } from './shell';
import { login } from './login';
import { users } from './users';
import { system } from './system';
import { security } from './security';
import { updates } from './updates';
import { errors } from './errors';
import { network } from './network';

export const fr: Record<string, string> = { ...common, ...shell, ...login, ...users, ...system, ...security, ...updates, ...errors, ...network };
