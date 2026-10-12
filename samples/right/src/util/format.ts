export interface User {
  id: number;
  name: string;
  email?: string;
}

export function formatId(user: User): string {
    const id = String(user.id);
    return id.padStart(4, "0");
}

export function displayName(user: User): string {
  const name = user.name.trim();
  if (name.length === 0) {
    return "Anonymous";
  }
  return name;
}

export function initials(user: User): string {
  return displayName(user)
    .split(/\s+/)
    .map((part) => part.charAt(0))
    .join("")
    .toUpperCase();
}

/** Friendly greeting, uses the e-mail when there is no name. */
export function greet(user: User): string {
  const who = user.name || user.email || "stranger";
  return `Hello, ${who}!`;
}

export const VERSION = "1.1.0";
