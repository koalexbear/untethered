"""GTK-only drag source for qa_portal_drop.py, running on its private bus."""
import os
import sys
from pathlib import Path

import gi
gi.require_version("Gtk","4.0")
gi.require_version("Gdk","4.0")
from gi.repository import Gdk, Gio, GLib, Gtk


def main():
    root=Path(sys.argv[1])
    paths=[root/"Portal shot.png",root/"Two notes.pdf"]
    bus=Gio.bus_get_sync(Gio.BusType.SESSION)
    dest=("org.freedesktop.portal.Documents","/org/freedesktop/portal/documents",
          "org.freedesktop.portal.FileTransfer")
    mime="application/vnd.portal.filetransfer"
    def call(method,args):
        return bus.call_sync(*dest,method,args,None,Gio.DBusCallFlags.NONE,5000,None).unpack()
    key,=call("StartTransfer",GLib.Variant("(a{sv})",({"autostop":GLib.Variant("b",False)},)))
    files=Gio.UnixFDList.new()
    for path in paths:
        with path.open("rb") as f: files.append(f.fileno())
    bus.call_with_unix_fd_list_sync(*dest,"AddFiles",GLib.Variant("(saha{sv})",(key,[0,1],{})),
                                    None,Gio.DBusCallFlags.NONE,5000,files,None)
    app=Gtk.Application(application_id="org.example.UntetheredPortalProof",flags=Gio.ApplicationFlags.NON_UNIQUE)
    def activate(app):
        win=Gtk.ApplicationWindow(application=app,title="Portal-only source")
        label=Gtk.Label(label="Drag the PNG and PDF from here")
        win.set_child(label)
        source=Gtk.DragSource.new()
        source.set_actions(Gdk.DragAction.COPY)
        provider=Gdk.ContentProvider.new_for_bytes(mime,GLib.Bytes.new(key.encode()))
        assert provider.ref_formats().get_mime_types()==[mime]
        source.set_content(provider)
        def finished(_source,drag,_delete):
            if drag.get_selected_action()!=Gdk.DragAction.COPY:
                return
            # A sender is entitled to close the grant at drag completion.
            call("StopTransfer",GLib.Variant("(s)",(key,)))
            # Model an application dropping temporary exports. The receiver
            # must still deliver the original files, not replacements by name.
            for path in paths:
                path.unlink()
                path.write_bytes(b"not the selected file")
            (root/"source-finished").touch()
        source.connect("drag-end",finished)
        label.add_controller(source)
        win.present()
        (root/"source-ready").touch()
    app.connect("activate",activate)
    app.run([])


if __name__=="__main__":
    main()
