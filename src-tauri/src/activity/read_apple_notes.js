function run() {
    const rows = [];
    for (const note of Application("Notes").notes()) {
        if (!note.passwordProtected()) rows.push([note.id(), note.name(), note.plaintext()]);
    }
    return JSON.stringify(rows);
}
