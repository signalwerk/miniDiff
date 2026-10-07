export interface User {
  id: number;
  name: string;
  email?: string;
}

/** Friendly greeting, uses the e-mail when there is no name. */
export function greet(user: User): string {
  const who = user.name || user.email || "stranger";
  return `Hello, ${who}!`;
}

export const VERSION = "1.1.0";
