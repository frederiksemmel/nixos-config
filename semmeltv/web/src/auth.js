const api = document.documentElement.dataset.api || "";

function toast(text, tone = "error") {
  window.dispatchEvent(new CustomEvent("semmeltv:toast", { detail: { text, tone } }));
}

function say(text) {
  const note = document.getElementById("sign-in-note");
  if (note) note.textContent = text;
  else if (text) toast(text);
}

function bytes(text) {
  const base64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64 + "===".slice((base64.length + 3) % 4);
  return Uint8Array.from(atob(padded), (char) => char.charCodeAt(0)).buffer;
}

function text(buffer) {
  let binary = "";
  for (const byte of new Uint8Array(buffer)) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function call(path, body, method = "POST") {
  let response;
  try {
    response = await fetch(`${api}${path}`, {
      method,
      headers: { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw Object.assign(new Error("The TV is not answering"), { status: 0 });
  }
  const answer = await response.json().catch(() => ({}));
  if (!response.ok || answer.ok === false) {
    throw Object.assign(new Error(answer.error || answer.message || `The TV answered ${response.status}`), {
      status: response.status,
      answer,
    });
  }
  return answer;
}

function next() {
  const wanted = new URLSearchParams(location.search).get("next");
  return wanted && wanted.startsWith("/") && !wanted.startsWith("//") && !wanted.startsWith("/_topcoat/") ? wanted : "/";
}

function arrive(where = next()) {
  location.assign(where);
}

function cancelled(error) {
  return error && (error.name === "NotAllowedError" || error.name === "AbortError");
}

function note(level, message, detail = "") {
  if (window.semmeltvReport) window.semmeltvReport(level, message, detail);
}

const FRESH_FOR = 4 * 60 * 1000;
const ready = new Map();

async function prepare(profile) {
  try {
    const started = await call("/api/auth/passkey/start", { profile });
    ready.set(profile, { ...started, at: Date.now() });
  } catch (error) {
    ready.delete(profile);
    note("warn", "passkey sign-in could not start", `profile ${profile}: ${error.message}`);
  }
}

function prepareAll() {
  for (const choice of document.querySelectorAll('.profile-choice[data-passkey="true"]')) prepare(Number(choice.dataset.profile));
}

async function passkeySignIn(profile) {
  let started = ready.get(profile);
  ready.delete(profile);
  if (!started || Date.now() - started.at > FRESH_FOR) {
    started = await call("/api/auth/passkey/start", { profile });
  }
  const { ceremony, options } = started;
  const publicKey = { ...options.publicKey, challenge: bytes(options.publicKey.challenge) };
  publicKey.allowCredentials = (publicKey.allowCredentials || []).map((credential) => ({ ...credential, id: bytes(credential.id) }));
  note("info", "passkey sign-in asks the device", `profile ${profile}`);
  const credential = await navigator.credentials.get({ publicKey });
  const response = credential.response;
  await call("/api/auth/passkey/finish", {
    ceremony,
    credential: {
      id: credential.id,
      rawId: text(credential.rawId),
      type: credential.type,
      response: {
        authenticatorData: text(response.authenticatorData),
        clientDataJSON: text(response.clientDataJSON),
        signature: text(response.signature),
        userHandle: response.userHandle ? text(response.userHandle) : null,
      },
      extensions: {},
    },
  });
  note("info", "passkey sign-in worked", `profile ${profile}`);
}

async function addPasskey() {
  const { ceremony, options } = await call("/api/auth/passkeys/start", {});
  const publicKey = {
    ...options.publicKey,
    challenge: bytes(options.publicKey.challenge),
    user: { ...options.publicKey.user, id: bytes(options.publicKey.user.id) },
  };
  publicKey.excludeCredentials = (publicKey.excludeCredentials || []).map((credential) => ({ ...credential, id: bytes(credential.id) }));
  const credential = await navigator.credentials.create({ publicKey });
  const response = credential.response;
  return call("/api/auth/passkeys/finish", {
    ceremony,
    credential: {
      id: credential.id,
      rawId: text(credential.rawId),
      type: credential.type,
      response: {
        attestationObject: text(response.attestationObject),
        clientDataJSON: text(response.clientDataJSON),
        transports: response.getTransports ? response.getTransports() : undefined,
      },
      extensions: {},
    },
  });
}

const passkeys = Boolean(window.PublicKeyCredential && navigator.credentials);
let chosen = null;

function askPassword(choice, note = "") {
  const form = document.getElementById("password-form");
  if (!form) return;
  chosen = choice;
  form.hidden = false;
  document.getElementById("password-name").textContent = choice.dataset.name;
  form.elements.username.value = choice.dataset.name;
  document.getElementById("password-passkey").hidden = !(passkeys && choice.dataset.passkey === "true");
  for (const other of document.querySelectorAll(".profile-choice")) other.classList.toggle("chosen", other === choice);
  say(note);
  form.elements.password.value = "";
  form.elements.password.focus();
}

async function tryPasskey(choice) {
  say("");
  const profile = Number(choice.dataset.profile);
  try {
    await passkeySignIn(profile);
    arrive();
  } catch (error) {
    note("warn", "passkey sign-in failed", `profile ${profile}: ${error.name}: ${error.message}`);
    prepare(profile);
    const why = cancelled(error)
      ? "The passkey didn't sign you in: it was cancelled, or this device has no passkey for you."
      : `The passkey didn't sign you in: ${error.message}.`;
    if (choice.dataset.password === "true") askPassword(choice, `${why} Use your password, or tap your name to try again.`);
    else say(`${why} Tap your name to try again.`);
  }
}

async function choose(choice) {
  const profile = Number(choice.dataset.profile);
  if (choice.getAttribute("aria-current") === "true") return arrive();
  if (choice.dataset.signedIn === "true" && choice.dataset.admin !== "true") {
    try {
      await call("/api/auth/switch", { profile });
      return arrive();
    } catch (error) {
      if (!(error.answer && error.answer.sign_in)) return say(error.message);
    }
  }
  if (passkeys && choice.dataset.passkey === "true") return tryPasskey(choice);
  if (choice.dataset.password === "true") return askPassword(choice);
  const who = choice.dataset.admin === "true" ? "the invite link from the TV" : "the invite link Frederik sends you";
  say(`${choice.dataset.name} isn't set up yet. Open ${who} once; it sets up Face ID and a password.`);
  for (const other of document.querySelectorAll(".profile-choice")) other.classList.toggle("chosen", other === choice);
}

document.addEventListener("click", (event) => {
  const choice = event.target.closest(".profile-choice");
  if (choice) {
    event.preventDefault();
    window.semmeltvAct({ key: `profile:${choice.dataset.profile}`, mark: choice, send: () => choose(choice) });
    return;
  }
  if (event.target.closest("#password-passkey") && chosen) {
    event.preventDefault();
    tryPasskey(chosen);
    return;
  }
  const out = event.target.closest("#sign-out");
  if (out) {
    event.preventDefault();
    window.semmeltvAct({
      key: "sign-out",
      mark: out,
      send: async () => {
        await call("/api/auth/sign-out", {});
        location.assign("/profiles");
      },
    });
    return;
  }
  const redeem = event.target.closest("#redeem");
  if (redeem) {
    event.preventDefault();
    window.semmeltvAct({
      key: "redeem",
      mark: redeem,
      send: async () => {
        await call(`/api/auth/invite/${encodeURIComponent(redeem.dataset.token)}`, {});
        location.assign("/account?welcome=1");
      },
    });
    return;
  }
  const add = event.target.closest("#add-passkey");
  if (add) {
    event.preventDefault();
    if (!passkeys) return toast("This browser can't make passkeys");
    window.semmeltvAct({
      key: "add-passkey",
      mark: add,
      send: async () => {
        try {
          await addPasskey();
        } catch (error) {
          note("warn", "adding a passkey failed", `${error.name}: ${error.message}`);
          if (cancelled(error)) return;
          throw error;
        }
        note("info", "passkey added");
        toast("Passkey saved", "ok");
        location.reload();
      },
    });
    return;
  }
  const invite = event.target.closest("[data-invite]");
  if (invite) {
    event.preventDefault();
    window.semmeltvAct({
      key: `invite:${invite.dataset.invite}`,
      mark: invite,
      send: async () => {
        const answer = await call(`/api/auth/members/${invite.dataset.invite}/invite`, {});
        const shown = invite.closest(".member").querySelector(".invite-link");
        shown.hidden = false;
        shown.textContent = answer.link;
        const message = `SemmelTV: open this link once to sign in as ${invite.dataset.name}. It works for ${answer.expires_in_days} days.`;
        if (navigator.share) {
          navigator.share({ title: "SemmelTV", text: message, url: answer.link }).catch(() => {});
        } else if (navigator.clipboard) {
          navigator.clipboard.writeText(answer.link).then(() => toast("Invite link copied", "ok"), () => {});
        }
      },
    });
    return;
  }
  const remove = event.target.closest("[data-remove]");
  if (remove) {
    event.preventDefault();
    if (!confirm(`Remove ${remove.dataset.name}? Their watch history and settings go too.`)) return;
    window.semmeltvAct({
      key: `remove:${remove.dataset.remove}`,
      mark: remove,
      send: async () => {
        await call(`/api/auth/members/${remove.dataset.remove}`, undefined, "DELETE");
        location.reload();
      },
    });
  }
});

document.addEventListener("submit", (event) => {
  const form = event.target;
  if (form.id === "password-form" && chosen) {
    event.preventDefault();
    const choice = chosen;
    window.semmeltvAct({
      key: "password",
      mark: form.querySelector("[type=submit]"),
      send: async () => {
        try {
          await call("/api/auth/password", { profile: Number(choice.dataset.profile), password: form.elements.password.value });
        } catch (error) {
          say(error.message);
          form.elements.password.select();
          return;
        }
        arrive();
      },
    });
  } else if (form.id === "password-set") {
    event.preventDefault();
    window.semmeltvAct({
      key: "password-set",
      mark: form.querySelector("[type=submit]"),
      send: async () => {
        const answer = await call("/api/auth/account/password", { password: form.elements.password.value });
        form.elements.password.value = "";
        toast(answer.message || "Password saved", "ok");
      },
    });
  } else if (form.id === "add-member") {
    event.preventDefault();
    window.semmeltvAct({
      key: "add-member",
      mark: form.querySelector("[type=submit]"),
      send: async () => {
        await call("/api/auth/members", { name: form.elements.name.value, role: form.elements.role.value });
        location.reload();
      },
    });
  }
});

if (new URLSearchParams(location.search).get("welcome") && document.getElementById("add-passkey")) {
  toast("You're signed in. Add a passkey and a password below.", "ok");
}

if (passkeys && document.querySelector(".profile-choice")) {
  prepareAll();
  setInterval(prepareAll, FRESH_FOR);
}
