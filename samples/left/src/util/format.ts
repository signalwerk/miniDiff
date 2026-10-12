export interface User {
  id: number;
  name: string;
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

export function greet(user: User): string {
  return "Hello, " + user.name;
}

export const VERSION = "1.0.0";
