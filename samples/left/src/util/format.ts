export interface User {
  id: number;
  name: string;
}

export function greet(user: User): string {
  return "Hello, " + user.name;
}

export const VERSION = "1.0.0";
