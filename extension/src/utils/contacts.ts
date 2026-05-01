/**
 * Address book / contacts persistence.
 *
 * Backed by chrome.storage.local with a localStorage fallback for
 * dev/test environments (mirrors the preferences.ts pattern).
 *
 * Each contact is keyed by its address (lowercased). Labels are
 * human-readable identifiers users assign to recipients they send to
 * frequently; notes are an optional free-form annotation.
 */

export interface Contact {
  /** Recipient address — case is preserved as the user typed it, but
   *  lookups are case-insensitive (see `findContact`). */
  address: string;
  /** Human-readable label, e.g. "Alice's hot wallet". */
  label: string;
  /** Optional free-form note. */
  note?: string;
  /** ms epoch when the contact was first saved. */
  addedAt: number;
}

const STORAGE_KEY = "evaporchain_contacts";

function isChromeStorageAvailable(): boolean {
  return (
    typeof chrome !== "undefined" &&
    typeof chrome.storage !== "undefined" &&
    typeof chrome.storage.local !== "undefined"
  );
}

function normalise(address: string): string {
  return address.trim().toLowerCase();
}

export async function loadContacts(): Promise<Contact[]> {
  try {
    if (isChromeStorageAvailable()) {
      return new Promise((resolve) => {
        chrome.storage.local.get(STORAGE_KEY, (result) => {
          const stored = result[STORAGE_KEY];
          resolve(Array.isArray(stored) ? (stored as Contact[]) : []);
        });
      });
    }
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as Contact[]) : [];
  } catch {
    return [];
  }
}

async function writeContacts(contacts: Contact[]): Promise<void> {
  try {
    if (isChromeStorageAvailable()) {
      return new Promise((resolve) => {
        chrome.storage.local.set({ [STORAGE_KEY]: contacts }, resolve);
      });
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(contacts));
  } catch {
    // Non-fatal — in-memory state still reflects the change
  }
}

/**
 * Save (insert or update) a contact. Address comparison is case-insensitive.
 * Returns the resulting contact list.
 */
export async function saveContact(contact: Contact): Promise<Contact[]> {
  const list = await loadContacts();
  const key = normalise(contact.address);
  const idx = list.findIndex((c) => normalise(c.address) === key);
  if (idx >= 0) {
    list[idx] = { ...list[idx], ...contact };
  } else {
    list.push({ ...contact, addedAt: contact.addedAt || Date.now() });
  }
  await writeContacts(list);
  return list;
}

export async function removeContact(address: string): Promise<Contact[]> {
  const key = normalise(address);
  const list = (await loadContacts()).filter((c) => normalise(c.address) !== key);
  await writeContacts(list);
  return list;
}

export async function findContact(address: string): Promise<Contact | null> {
  const key = normalise(address);
  const list = await loadContacts();
  return list.find((c) => normalise(c.address) === key) ?? null;
}

/** Synchronous lookup against an in-memory list — used by SendScreen for autocomplete. */
export function findContactSync(list: Contact[], address: string): Contact | null {
  const key = normalise(address);
  return list.find((c) => normalise(c.address) === key) ?? null;
}
