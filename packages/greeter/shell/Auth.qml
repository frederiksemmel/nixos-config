pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Services.Greetd

// Drives one greetd login: the password typed before the first secret PAM
// prompt answers it, any further prompt (expired password, OTP) is shown to the
// user and answered from the input field.
Singleton {
    id: auth

    property string password: ""
    // A PAM prompt the user has to answer; empty while the plain password suffices.
    property string prompt: ""
    property bool promptEcho: false
    property string message: ""
    property bool messageIsError: false
    property bool busy: false
    property bool leaving: false

    property var pending: null

    signal rejected

    function submit() {
        if (busy || leaving)
            return;
        if (!Greetd.available) {
            show("greetd is not running", true);
            return;
        }
        if (prompt !== "") {
            const response = password;
            prompt = "";
            password = "";
            busy = true;
            Greetd.respond(response);
            return;
        }
        if (password === "")
            return;
        pending = password;
        password = "";
        busy = true;
        show("", false);
        Greetd.createSession(Config.user);
    }

    function cancel() {
        if (Greetd.state === GreetdState.Authenticating)
            Greetd.cancelSession();
        pending = null;
        prompt = "";
        password = "";
        busy = false;
        show("", false);
    }

    function show(text, isError) {
        message = text;
        messageIsError = isError;
    }

    function power(command) {
        leaving = true;
        Quickshell.execDetached(command);
    }

    Connections {
        target: Greetd

        function onAuthMessage(message, error, responseRequired, echoResponse) {
            if (responseRequired && !echoResponse && auth.pending !== null) {
                const response = auth.pending;
                auth.pending = null;
                Greetd.respond(response);
            } else if (responseRequired) {
                auth.busy = false;
                auth.password = "";
                auth.prompt = message.trim().replace(/:$/, "");
                auth.promptEcho = echoResponse;
            } else {
                auth.show(message, error);
            }
        }

        function onAuthFailure(message) {
            auth.pending = null;
            auth.prompt = "";
            auth.password = "";
            auth.busy = false;
            auth.show(message.includes("AUTH_ERR") ? "Authentication failed" : message, true);
            auth.rejected();
        }

        function onError(error) {
            auth.pending = null;
            auth.prompt = "";
            auth.busy = false;
            auth.show(error, true);
        }

        function onReadyToLaunch() {
            auth.leaving = true;
            launchTimer.start();
        }
    }

    // greetd wants the greeter gone right after launch(), so the fade-out
    // happens before it.
    Timer {
        id: launchTimer

        interval: 300
        onTriggered: Greetd.launch(Config.sessionCommand, Config.sessionEnvironment)
    }
}
