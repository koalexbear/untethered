import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import QtMultimedia

ApplicationWindow {
    id: window
    width: 1150
    height: 740
    minimumWidth: 760
    minimumHeight: 520
    visible: true
    title: "untethered"
    color: "#1c2023"
    palette.window: window.color
    palette.base: window.fieldSurface
    palette.text: window.foreground
    palette.windowText: window.foreground
    palette.button: window.hoverSurface
    palette.buttonText: window.foreground
    palette.placeholderText: window.muted
    palette.highlight: "#34524a"
    palette.highlightedText: window.foreground
    readonly property color foreground: "#f0edf3"
    readonly property color muted: "#a9b2ba"
    readonly property color accent: "#9be1c9"
    readonly property color activeSurface: "#30413f"
    readonly property color hoverSurface: "#2a3237"
    readonly property color sidebarSurface: "#171c20"
    readonly property color railSurface: "#121619"
    readonly property color fieldSurface: "#242b30"
    readonly property color divider: "#313a40"
    readonly property string directContext: "direct"
    readonly property string contextName: contextId === directContext ? "Direct messages"
        : contextId === "" ? "All rooms" : (spaces.find(space => space.id === contextId)?.name ?? "Space")
    readonly property var roomIndex: chat.roomList
    readonly property var spaces: roomIndex.filter(room => room.space)
    readonly property var selectedRoom: roomIndex.find(room => room.id === chat.selected)
    readonly property var visibleRooms: roomsFor(contextId)
    property string contextId: ""
    property var lastRooms: ({})
    property var drafts: ({})
    property string draftRoom: ""
    property bool hadSession: false
    property var replyDrafts: ({})
    property var replyTarget: null
    property var imageDrafts: ({})
    property url imageAttachment: ""
    readonly property bool sendingImage: chat.selected !== "" && chat.imageUploadRoom === chat.selected
    readonly property bool canAttachImage: chat.ready && !chat.loggingOut
        && chat.selected !== "" && !sendingImage

    property bool showCall: false
    readonly property bool callOpen: chat.nativeCall
    property bool callOutputMuted: false
    function openCallAudioSettings() {
        openCallPane()
        if (chat.nativeConnected)
            nativeAudioSettings.open()
    }

    function openCallPane() {
        if (!callOpen)
            chat.openCall()
        showCall = callOpen
    }

    onCallOpenChanged: {
        if (!callOpen) {
            showCall = false
            callOutputMuted = false
            nativeAudioSettings.close()
            nativeSharePicker.close()
        }
    }

    function startReply(message) {
        replyTarget = message
        composer.forceActiveFocus()
    }

    function roomsFor(context) {
        return roomIndex.filter(room => !room.space && (context === ""
            || (context === directContext ? room.dm : chat.inSpace(context, room.id))))
    }

    function navigate(context, roomId) {
        if (chat.selected !== "")
            lastRooms[contextId] = chat.selected

        const candidates = roomsFor(context)
        const target = candidates.find(room => room.id === roomId)
            ?? candidates.find(room => room.id === lastRooms[context])
            ?? candidates[0]
        contextId = context
        chat.openRoom(target?.id ?? "")
    }

    function openSearchResult(room) {
        if (room.space) {
            navigate(room.id)
        } else if (room.dm) {
            navigate(directContext, room.id)
        } else {
            const space = spaces.find(candidate => candidate.id === contextId
                && chat.inSpace(candidate.id, room.id))
                ?? spaces.find(candidate => chat.inSpace(candidate.id, room.id))
            navigate(space?.id ?? "", room.id)
        }
    }

    Connections {
        target: chat
        function onChanged() {
            if (!chat.loggedIn) {
                if (!window.hadSession)
                    return
                window.hadSession = false
                imageViewer.close()
                attachmentPicker.close()
                logoutConfirmation.close()
                settings.close()
                switcher.close()
                window.contextId = ""
                window.lastRooms = ({})
                window.drafts = ({})
                window.draftRoom = ""
                window.replyDrafts = ({})
                window.replyTarget = null
                window.imageDrafts = ({})
                window.imageAttachment = ""
                composer.clear()
                recovery.clear()
                password.clear()
                return
            }
            window.hadSession = true
            if (window.draftRoom !== chat.selected) {
                window.showCall = false
                imageViewer.close()
                if (window.draftRoom !== "") {
                    window.drafts[window.draftRoom] = composer.text
                    window.replyDrafts[window.draftRoom] = window.replyTarget
                    window.imageDrafts[window.draftRoom] = window.imageAttachment
                }
                window.draftRoom = chat.selected
                composer.text = window.drafts[chat.selected] ?? ""
                window.replyTarget = window.replyDrafts[chat.selected] ?? null
                window.imageAttachment = window.imageDrafts[chat.selected] ?? ""
            }
            Qt.callLater(window.reconcileNavigation)
        }

        function onImageSent(room) {
            delete window.drafts[room]
            delete window.replyDrafts[room]
            delete window.imageDrafts[room]
            if (room === chat.selected) {
                composer.clear()
                window.replyTarget = null
                window.imageAttachment = ""
            }
        }
    }

    function reconcileNavigation() {
        if (!chat.loggedIn)
            return
        if (contextId !== "" && contextId !== directContext
                && !spaces.some(space => space.id === contextId)) {
            navigate("")
        } else if (!visibleRooms.some(room => room.id === chat.selected)) {
            navigate(contextId)
        }
    }

    component ProfileButton: ToolButton {
        id: profileButton
        required property string symbol
        property bool destructive: false
        Layout.preferredWidth: 30
        Layout.preferredHeight: 32
        padding: 6

        background: Rectangle {
            radius: 6
            color: profileButton.checked ? "#46313b"
                : profileButton.hovered || profileButton.visualFocus ? window.hoverSurface : "transparent"
            border.color: profileButton.visualFocus ? window.accent : "transparent"
        }

        contentItem: Canvas {
            id: profileIcon
            readonly property color ink: (profileButton.checked || profileButton.destructive) && profileButton.enabled ? "#f095a4"
                : profileButton.enabled ? window.palette.text : window.muted
            onInkChanged: requestPaint()
            Connections {
                target: profileButton
                function onCheckedChanged() { profileIcon.requestPaint() }
            }
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                const size = Math.min(width, height)
                ctx.translate((width - size) / 2, (height - size) / 2)
                ctx.scale(size / 20, size / 20)
                ctx.strokeStyle = ink
                ctx.lineWidth = 1.7
                ctx.lineCap = "round"
                ctx.lineJoin = "round"

                ctx.beginPath()
                switch (profileButton.symbol) {
                case "microphone":
                    ctx.roundedRect(7, 2, 6, 11, 3, 3)
                    ctx.moveTo(4, 10)
                    ctx.quadraticCurveTo(4, 15, 10, 15)
                    ctx.quadraticCurveTo(16, 15, 16, 10)
                    ctx.moveTo(10, 15)
                    ctx.lineTo(10, 18)
                    ctx.moveTo(7, 18)
                    ctx.lineTo(13, 18)
                    if (profileButton.checked) {
                        ctx.moveTo(3, 17)
                        ctx.lineTo(17, 3)
                    }
                    break
                case "call":
                    ctx.roundedRect(2, 4, 16, 11, 2, 2)
                    ctx.moveTo(7, 18)
                    ctx.lineTo(13, 18)
                    ctx.moveTo(10, 15)
                    ctx.lineTo(10, 18)
                    break
                case "camera":
                    ctx.roundedRect(2, 5, 11, 10, 2, 2)
                    ctx.moveTo(13, 8)
                    ctx.lineTo(18, 5)
                    ctx.lineTo(18, 15)
                    ctx.lineTo(13, 12)
                    break
                case "share":
                    ctx.roundedRect(2, 3, 16, 12, 2, 2)
                    ctx.moveTo(6, 18)
                    ctx.lineTo(14, 18)
                    ctx.moveTo(10, 12)
                    ctx.lineTo(10, 6)
                    ctx.moveTo(7, 9)
                    ctx.lineTo(10, 6)
                    ctx.lineTo(13, 9)
                    break
                case "audio":
                    ctx.moveTo(2, 6)
                    ctx.lineTo(18, 6)
                    ctx.moveTo(7, 3)
                    ctx.lineTo(7, 9)
                    ctx.moveTo(2, 14)
                    ctx.lineTo(18, 14)
                    ctx.moveTo(13, 11)
                    ctx.lineTo(13, 17)
                    break
                case "hangup":
                    ctx.moveTo(2, 11)
                    ctx.quadraticCurveTo(10, 4, 18, 11)
                    ctx.lineTo(18, 14)
                    ctx.lineTo(14, 14)
                    ctx.lineTo(14, 11)
                    ctx.quadraticCurveTo(10, 9, 6, 11)
                    ctx.lineTo(6, 14)
                    ctx.lineTo(2, 14)
                    ctx.closePath()
                    break
                case "headphones":
                    ctx.moveTo(3, 13)
                    ctx.lineTo(3, 9)
                    ctx.arc(10, 9, 7, Math.PI, 0)
                    ctx.lineTo(17, 13)
                    ctx.roundedRect(3, 11, 4, 6, 1.5, 1.5)
                    ctx.roundedRect(13, 11, 4, 6, 1.5, 1.5)
                    break
                case "settings":
                    for (let i = 0; i < 32; i++) {
                        const angle = i * Math.PI / 16
                        const radius = i % 4 < 2 ? 8 : 6.2
                        const x = 10 + radius * Math.cos(angle)
                        const y = 10 + radius * Math.sin(angle)
                        if (i === 0) ctx.moveTo(x, y)
                        else ctx.lineTo(x, y)
                    }
                    ctx.closePath()
                    ctx.moveTo(12.7, 10)
                    ctx.arc(10, 10, 2.7, 0, 2 * Math.PI)
                    break
                }
                ctx.stroke()
            }
        }
    }

    component Avatar: Rectangle {
        id: avatar
        property url source
        property string name: ""
        property string fallbackText: Array.from(name.trim()).slice(0, 2).join("").toUpperCase()
        radius: width / 3
        color: window.activeSurface

        Label {
            anchors.centerIn: parent
            visible: picture.status !== Image.Ready
            text: avatar.fallbackText
            textFormat: Text.PlainText
            color: window.accent
            font.pixelSize: Math.max(10, avatar.width * 0.36)
            font.bold: true
        }
        Image {
            id: picture
            anchors.fill: parent
            anchors.margins: 2
            source: avatar.source
            asynchronous: true
            visible: status === Image.Ready
            fillMode: Image.PreserveAspectFit
        }
    }

    component NavigationButton: ItemDelegate {
        id: navigationButton
        property bool centered: false
        font.pixelSize: 14
        property url avatarSource
        property string avatarName: ""
        property bool showAvatar: false
        contentItem: RowLayout {
            spacing: 8
            Avatar {
                visible: navigationButton.showAvatar
                source: navigationButton.avatarSource
                name: navigationButton.avatarName
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
            }
            Label {
                Layout.fillWidth: true
                text: navigationButton.text
                textFormat: Text.PlainText
                color: navigationButton.centered && navigationButton.highlighted ? window.accent
                    : navigationButton.highlighted || navigationButton.font.bold || navigationButton.hovered
                        ? window.palette.text : window.muted
                font: navigationButton.font
                elide: Text.ElideRight
                horizontalAlignment: navigationButton.centered ? Text.AlignHCenter : Text.AlignLeft
                verticalAlignment: Text.AlignVCenter
            }
        }
        background: Rectangle {
            radius: 8
            color: navigationButton.highlighted ? window.activeSurface
                : navigationButton.hovered || navigationButton.visualFocus ? window.hoverSurface : "transparent"
            border.width: navigationButton.visualFocus ? 1 : 0
            border.color: window.accent
        }
    }

    Shortcut {
        sequence: "Ctrl+K"
        enabled: chat.loggedIn && !settings.opened && !imageViewer.opened
        onActivated: switcher.open()
    }

    Shortcut {
        sequence: "Ctrl+,"
        enabled: chat.loggedIn && !switcher.opened && !settings.opened && !imageViewer.opened
        onActivated: settings.open()
    }

    Dialog {
        id: settings
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(window.width - 48, 500)
        padding: 20
        title: "User settings"
        modal: true
        standardButtons: Dialog.Close
        onClosed: recovery.clear()

        contentItem: ColumnLayout {
            spacing: 14
            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                Avatar {
                    source: chat.profileAvatar
                    name: chat.profileName.startsWith("@") ? chat.profileName.slice(1) : chat.profileName
                    Layout.preferredWidth: 48
                    Layout.preferredHeight: 48
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4
                    Label {
                        text: chat.profileName
                        textFormat: Text.PlainText
                        font.pixelSize: 18
                        font.weight: Font.DemiBold
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                    }
                    TextEdit {
                        text: chat.profile
                        textFormat: TextEdit.PlainText
                        readOnly: true
                        selectByMouse: true
                        wrapMode: TextEdit.Wrap
                        color: window.muted
                        selectionColor: window.palette.highlight
                        selectedTextColor: window.palette.highlightedText
                        Layout.fillWidth: true
                        Accessible.name: "Matrix account ID"
                    }
                }
            }
            Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
            Label {
                text: "Device verification"
                font.weight: Font.DemiBold
            }
            Label {
                text: chat.verified ? "This device is verified" : "This device is not verified"
                color: chat.verified ? window.accent : window.muted
            }
            Label {
                visible: chat.deviceId !== ""
                text: "Device ID: " + chat.deviceId
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                color: window.muted
                Layout.fillWidth: true
            }
            Label {
                visible: !chat.verified
                text: "Use your Matrix recovery key to verify this device."
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            RowLayout {
                visible: !chat.verified
                Layout.fillWidth: true
                TextField {
                    id: recovery
                    placeholderText: "Matrix recovery key"
                    Accessible.name: placeholderText
                    echoMode: TextInput.Password
                    enabled: chat.ready
                    Layout.fillWidth: true
                    onAccepted: if (verifyDevice.enabled) verifyDevice.clicked()
                }
                Button {
                    id: verifyDevice
                    text: "Verify device"
                    enabled: chat.ready && recovery.text.trim().length > 0
                    onClicked: {
                        chat.verify(recovery.text)
                        recovery.clear()
                    }
                }
            }
            Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
            Button {
                text: "Call Audio Settings"
                enabled: chat.nativeConnected
                onClicked: {
                    settings.close()
                    window.openCallAudioSettings()
                }
            }
            Button {
                text: chat.loggingOut ? "Signing out…" : "Sign out"
                enabled: chat.ready && !chat.loggingOut
                onClicked: logoutConfirmation.open()
            }
            Label {
                visible: text.length > 0
                text: chat.status
                textFormat: Text.PlainText
                wrapMode: Text.WordWrap
                color: window.muted
                Layout.fillWidth: true
            }
        }
    }

    FileDialog {
        id: attachmentPicker
        title: "Attach an image"
        fileMode: FileDialog.OpenFile
        nameFilters: ["Images (*.png *.jpg *.jpeg *.webp *.gif *.bmp *.avif *.heic *.tif *.tiff)", "All files (*)"]
        property string room
        onAccepted: {
            if (room === chat.selected && window.canAttachImage) {
                if (chat.canAttachImage(selectedFile)) {
                    window.imageAttachment = selectedFile
                    composer.forceActiveFocus()
                } else {
                    invalidImage.open()
                }
            }
        }
    }

    Dialog {
        id: invalidImage
        implicitWidth: Math.min(window.width - 48, 420)
        parent: Overlay.overlay
        anchors.centerIn: parent
        title: "Image unavailable"
        modal: true
        standardButtons: Dialog.Ok
        contentItem: Label { text: "The selected file could not be opened as an image." }
    }

    Dialog {
        id: imageViewer
        parent: Overlay.overlay
        anchors.centerIn: parent
        title: "Image"
        width: Math.min(window.width - 48, 1050)
        height: Math.min(window.height - 48, 850)
        modal: true
        standardButtons: Dialog.Close
        property url imageSource
        onClosed: imageSource = ""

        contentItem: Item {
            Image {
                id: enlargedImage
                anchors.fill: parent
                source: imageViewer.imageSource
                asynchronous: true
                fillMode: Image.PreserveAspectFit
                sourceSize.width: 1920
                sourceSize.height: 1920
            }
            Label {
                anchors.centerIn: parent
                visible: enlargedImage.status !== Image.Ready
                color: window.muted
                text: enlargedImage.status === Image.Loading ? "Loading image…" : "Image unavailable"
            }
        }
    }

    Dialog {
        id: logoutConfirmation
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(window.width - 48, 440)
        padding: 20
        title: "Sign out of Matrix?"
        modal: true
        onAccepted: chat.logout()

        contentItem: Label {
            text: "Signing out removes this device’s local encryption keys and cached messages. Keep your Matrix recovery key so you can recover backed-up keys after signing in again."
            wrapMode: Text.WordWrap
        }

        footer: DialogButtonBox {
            Button {
                text: "Cancel"
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
            }
            Button {
                text: "Sign out"
                enabled: chat.ready && !chat.loggingOut
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
            }
        }
    }

    Dialog {
        id: switcher
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(window.width - 48, 520)
        modal: true
        title: "Jump to a room or space"
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

        function acceptResult(room) {
            if (room) {
                close()
                window.openSearchResult(room)
            }
        }

        onOpened: {
            query.clear()
            results.currentIndex = 0
            query.forceActiveFocus()
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            TextField {
                id: query
                Layout.fillWidth: true
                placeholderText: "Search spaces, channels and DMs"
                Accessible.name: placeholderText
                onTextChanged: results.currentIndex = 0
                Keys.onUpPressed: results.currentIndex = Math.max(0, results.currentIndex - 1)
                Keys.onDownPressed: results.currentIndex = Math.min(results.count - 1, results.currentIndex + 1)
                onAccepted: switcher.acceptResult(results.model[results.currentIndex])
            }
            ListView {
                id: results
                Layout.fillWidth: true
                Layout.preferredHeight: 280
                clip: true
                model: window.roomIndex.filter(room => room.name.toLowerCase().includes(query.text.trim().toLowerCase()))
                delegate: NavigationButton {
                    required property var modelData
                    required property int index
                    width: ListView.view.width
                    text: (modelData.space ? "Space  ·  " : modelData.dm ? "DM  ·  " : "#  ") + modelData.name
                    highlighted: results.currentIndex === index
                    onClicked: switcher.acceptResult(modelData)
                }
                ScrollBar.vertical: ScrollBar {}
                Label {
                    anchors.centerIn: parent
                    visible: results.count === 0
                    color: window.muted
                    text: "No matching rooms or spaces"
                }
            }
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: 350
        spacing: 12
        visible: !chat.loggedIn

        Label { text: window.title; font.pixelSize: 34; font.weight: Font.DemiBold }
        Label { text: chat.status; wrapMode: Text.WordWrap; Layout.fillWidth: true; color: window.muted }
        TextField { id: server; placeholderText: "https://matrix.example.org"; Layout.fillWidth: true; enabled: chat.ready }
        TextField { id: user; placeholderText: "Matrix username"; Layout.fillWidth: true; enabled: chat.ready }
        TextField {
            id: password
            placeholderText: "Matrix password"
            echoMode: TextInput.Password
            Layout.fillWidth: true
            enabled: chat.ready
            onAccepted: signIn.clicked()
        }
        Button {
            id: signIn
            text: "Sign in"
            enabled: chat.ready && password.text.length > 0
            onClicked: {
                chat.login(server.text, user.text, password.text)
                password.clear()
            }
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0
        visible: chat.loggedIn

        Rectangle {
            Layout.preferredWidth: window.width < 900 ? 270 : 334
            Layout.fillHeight: true
            color: window.sidebarSurface

            ColumnLayout {
                anchors.fill: parent
                spacing: 0
                RowLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: 0
                    Rectangle {
                        Layout.preferredWidth: 62
                        Layout.fillHeight: true
                        color: window.railSurface
                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 10
                            spacing: 6
                            NavigationButton {
                                text: "DM"
                                centered: true
                                font.bold: true
                                Layout.fillWidth: true
                                highlighted: window.contextId === window.directContext
                                Accessible.name: "Direct messages"
                                ToolTip.text: Accessible.name
                                ToolTip.visible: hovered
                                onClicked: window.navigate(window.directContext)
                            }
                            NavigationButton {
                                text: "All"
                                centered: true
                                font.bold: true
                                Layout.fillWidth: true
                                highlighted: window.contextId === ""
                                Accessible.name: "All rooms"
                                ToolTip.text: Accessible.name
                                ToolTip.visible: hovered
                                onClicked: window.navigate("")
                            }
                            Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
                            ListView {
                                id: spaceList
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                clip: true
                                spacing: 6
                                model: window.spaces
                                delegate: NavigationButton {
                                    required property var modelData
                                    width: spaceList.width
                                    height: 42
                                    centered: true
                                    font.bold: true
                                    padding: 4
                                    Rectangle {
                                        anchors.left: parent.left
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 3
                                        height: 20
                                        radius: 1.5
                                        visible: parent.highlighted
                                        color: window.accent
                                    }
                                    contentItem: Avatar {
                                        name: modelData.name
                                        source: modelData.avatar ?? ""
                                    }
                                    highlighted: window.contextId === modelData.id
                                    Accessible.name: modelData.name
                                    ToolTip.text: modelData.name
                                    ToolTip.visible: hovered
                                    onClicked: window.navigate(modelData.id)
                                }
                            }
                        }
                    }
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: window.sidebarSurface
                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 10
                            spacing: 6
                            Label {
                                text: window.contextName
                                textFormat: Text.PlainText
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                                Layout.leftMargin: 8
                                Layout.preferredHeight: 38
                                verticalAlignment: Text.AlignVCenter
                                font.pixelSize: 16
                                font.weight: Font.DemiBold
                            }
                            Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
                            ListView {
                                id: rooms
                                Layout.fillHeight: true
                                Layout.fillWidth: true
                                clip: true
                                spacing: 2
                                model: window.visibleRooms
                                delegate: Column {
                                    required property var modelData
                                    id: roomRow
                                    width: rooms.width

                                    NavigationButton {
                                        width: parent.width
                                        height: roomRow.modelData.dm ? 38 : 32
                                        highlighted: chat.selected === roomRow.modelData.id
                                        avatarSource: roomRow.modelData.avatar ?? ""
                                        avatarName: roomRow.modelData.name
                                        showAvatar: roomRow.modelData.dm || avatarSource.toString() !== ""
                                        text: (roomRow.modelData.unread ? "•  " : "") + (showAvatar ? "" : "#  ") + roomRow.modelData.name
                                        font.bold: roomRow.modelData.unread
                                        Accessible.name: roomRow.modelData.name + (roomRow.modelData.unread ? ", unread" : "")
                                        onClicked: window.navigate(window.contextId, roomRow.modelData.id)
                                    }

                                    Repeater {
                                        model: roomRow.modelData.id === chat.callRoomId ? chat.callParticipants : []

                                        delegate: Item {
                                            required property var modelData
                                            id: callParticipant
                                            width: roomRow.width
                                            height: 28
                                            Accessible.role: Accessible.StaticText
                                            Accessible.name: modelData.name + (modelData.local ? ", you" : "")

                                            RowLayout {
                                                anchors.fill: parent
                                                anchors.leftMargin: 24
                                                anchors.rightMargin: 8
                                                spacing: 8

                                                Avatar {
                                                    Layout.preferredWidth: 20
                                                    Layout.preferredHeight: 20
                                                    name: callParticipant.modelData.name
                                                    source: callParticipant.modelData.avatar
                                                }
                                                Label {
                                                    Layout.fillWidth: true
                                                    text: callParticipant.Accessible.name
                                                    textFormat: Text.PlainText
                                                    elide: Text.ElideRight
                                                    color: window.muted
                                                    font.pixelSize: 13
                                                }
                                            }
                                        }
                                    }
                                }
                                ScrollBar.vertical: ScrollBar {}
                                Label {
                                    anchors.centerIn: parent
                                    visible: rooms.count === 0
                                    color: window.muted
                                    text: window.contextId === window.directContext ? "No direct messages" : "No rooms"
                                }
                            }
                        }
                    }
                }
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: window.callOpen ? 90 : 54
                    Layout.leftMargin: 8
                    Layout.rightMargin: 8
                    Layout.bottomMargin: 8
                    Layout.topMargin: 4
                    radius: 9
                    color: window.fieldSurface
                    border.color: window.divider

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 6
                        anchors.topMargin: 5
                        anchors.bottomMargin: 5
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            spacing: 4

                            Avatar {
                                source: chat.profileAvatar
                                name: chat.profileName.startsWith("@") ? chat.profileName.slice(1) : chat.profileName
                                Layout.preferredWidth: 32
                                Layout.preferredHeight: 32
                                Rectangle {
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    width: 12
                                    height: 12
                                    radius: width / 2
                                    border.width: 2
                                    border.color: window.fieldSurface
                                    color: chat.ready ? "#69be95" : window.muted
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.leftMargin: 4
                                spacing: 1

                                Label {
                                    text: chat.profileName
                                    textFormat: Text.PlainText
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                    font.pixelSize: 14
                                    ToolTip.text: chat.profile
                                    ToolTip.visible: profileHover.hovered
                                    HoverHandler { id: profileHover }
                                }
                                Label {
                                    text: chat.ready ? "Connected" : "Disconnected"
                                    color: window.muted
                                    font.pixelSize: 11
                                }
                            }

                            Item {
                                visible: !window.callOpen
                                Layout.preferredWidth: 98
                                Layout.preferredHeight: 32

                                RowLayout {
                                    id: idleCallActions
                                    anchors.fill: parent
                                    spacing: 4
                                }
                            }

                            ProfileButton {
                                symbol: "settings"
                                Accessible.name: "User settings"
                                ToolTip.text: "User settings · Ctrl+,"
                                ToolTip.visible: hovered
                                onClicked: settings.open()
                            }
                        }

                        RowLayout {
                            id: activeCallActions
                            visible: window.callOpen
                            Layout.alignment: Qt.AlignHCenter
                            spacing: 4

                            ProfileButton {
                                parent: window.callOpen ? activeCallActions : idleCallActions
                                symbol: "microphone"
                                enabled: chat.nativeConnected
                                checked: chat.nativeMuted
                                Accessible.name: checked ? "Unmute microphone" : "Mute microphone"
                                ToolTip.text: window.callOpen ? Accessible.name : "No active call"
                                ToolTip.visible: hovered
                                onClicked: chat.toggleNativeMicrophone()
                            }
                            ProfileButton {
                                parent: window.callOpen ? activeCallActions : idleCallActions
                                symbol: "call"
                                enabled: window.callOpen
                                Accessible.name: "Return to call"
                                ToolTip.text: Accessible.name
                                ToolTip.visible: hovered
                                onClicked: window.openCallPane()
                            }
                            ProfileButton {
                                parent: window.callOpen ? activeCallActions : idleCallActions
                                symbol: "headphones"
                                enabled: window.callOpen
                                checked: window.callOutputMuted
                                Accessible.name: checked ? "Unmute call audio" : "Mute call audio"
                                ToolTip.text: Accessible.name
                                ToolTip.visible: hovered
                                onClicked: {
                                    window.callOutputMuted = !window.callOutputMuted
                                    if (chat.nativeCall)
                                        chat.setNativeOutputMuted(window.callOutputMuted)
                                }
                            }
                            ProfileButton {
                                symbol: "camera"
                                enabled: chat.nativeConnected
                                Accessible.name: chat.nativeCamera ? "Turn camera off" : "Turn camera on"
                                ToolTip.text: Accessible.name
                                ToolTip.visible: hovered
                                onClicked: chat.toggleNativeCamera()
                            }
                            ProfileButton {
                                symbol: "share"
                                enabled: chat.nativeConnected
                                Accessible.name: chat.nativeSharing ? "Stop sharing" : "Share screen"
                                ToolTip.text: Accessible.name
                                ToolTip.visible: hovered
                                onClicked: {
                                    if (chat.nativeSharing) {
                                        chat.stopNativeShare()
                                        return
                                    }
                                    const sources = chat.nativeShareSources()
                                    if (sources.portal) {
                                        chat.startNativeShare(-1, false)
                                    } else {
                                        nativeSharePicker.sources = sources
                                        nativeSharePicker.open()
                                    }
                                }
                            }
                            ProfileButton {
                                symbol: "audio"
                                enabled: chat.nativeConnected
                                Accessible.name: "Call audio settings"
                                ToolTip.text: Accessible.name
                                ToolTip.visible: hovered
                                onClicked: nativeAudioSettings.open()
                            }
                            ProfileButton {
                                symbol: "hangup"
                                destructive: true
                                Accessible.name: "Leave call"
                                ToolTip.text: Accessible.name
                                ToolTip.visible: hovered
                                onClicked: chat.closeCall()
                            }
                        }

                    }
                }
            }
        }
        Rectangle { Layout.fillHeight: true; Layout.preferredWidth: 1; color: window.divider }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: window.color

            ColumnLayout {
                anchors.fill: parent
                spacing: 0
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 52
                    color: window.color
                    Rectangle {
                        anchors.bottom: parent.bottom
                        width: parent.width
                        height: 1
                        color: window.divider
                    }
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 20
                        anchors.rightMargin: 18
                        Label {
                            visible: !window.showCall && !!window.selectedRoom && !window.selectedRoom.dm
                            text: "#"
                            color: window.accent
                            font.pixelSize: 18
                            Accessible.ignored: true
                        }
                        Label {
                            text: window.showCall ? chat.callRoomName : (chat.roomName || "Choose a room")
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                            font.pixelSize: 17
                            font.weight: Font.DemiBold
                        }
                        Label { text: "Encrypted"; color: window.accent; visible: !window.showCall && chat.encrypted && window.width > 1050 }
                        NavigationButton {
                            text: "Find"
                            Accessible.name: "Find a conversation"
                            ToolTip.text: "Quick switch · Ctrl+K"
                            ToolTip.visible: hovered
                            onClicked: switcher.open()
                        }
                        NavigationButton {
                            text: window.showCall ? "Back to chat" : "Call"
                            Accessible.name: window.showCall ? "Back to chat" : "Open call in this room"
                            visible: chat.selected !== "" || window.showCall
                            enabled: window.showCall || (chat.ready && !chat.loggingOut)
                            onClicked: {
                                if (window.showCall)
                                    window.showCall = false
                                else
                                    window.openCallPane()
                            }
                        }
                        NavigationButton { text: "History"; Accessible.name: "Load older messages"; visible: chat.selected !== "" && !window.showCall; enabled: chat.ready; onClicked: chat.paginate(true) }
                    }
                }
                ColumnLayout {
                    visible: window.showCall && chat.nativeCall
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.margins: 12
                    spacing: 12

                    Label {
                        Layout.fillWidth: true
                        text: chat.callTransportStatus
                        color: window.muted
                        wrapMode: Text.WordWrap
                    }
                    GridView {
                        id: nativeVideoGrid
                        visible: count > 0
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        cellWidth: width / (count > 1 && width > 640 ? 2 : 1)
                        cellHeight: Math.min(height, cellWidth * 9 / 16)
                        clip: true
                        model: chat.nativeVideoTiles

                        delegate: Rectangle {
                            required property var modelData
                            width: nativeVideoGrid.cellWidth - 8
                            height: nativeVideoGrid.cellHeight - 8
                            radius: 10
                            color: "#14171a"

                            VideoOutput {
                                id: nativeTileVideo
                                anchors.fill: parent
                                fillMode: VideoOutput.PreserveAspectFit
                                Component.onCompleted: chat.setNativeVideoSink(modelData.id, videoSink)
                            }
                            Label {
                                anchors.left: parent.left
                                anchors.bottom: parent.bottom
                                anchors.margins: 10
                                padding: 6
                                text: modelData.name + (modelData.sharing ? " · Screen" : "")
                                textFormat: Text.PlainText
                                background: Rectangle { color: "#c0212428"; radius: 6 }
                            }
                        }
                    }
                    Item {
                        visible: nativeVideoGrid.count === 0
                        Layout.fillHeight: true
                    }

                }

                Dialog {
                    id: nativeSharePicker
                    parent: Overlay.overlay
                    anchors.centerIn: parent
                    width: Math.min(window.width - 32, 460)
                    title: "Share a screen or window"
                    modal: true
                    standardButtons: Dialog.Cancel
                    property var sources: ({ screens: [], windows: [] })

                    ColumnLayout {
                        TabBar {
                            id: nativeShareTab
                            Layout.fillWidth: true
                            TabButton { text: "Screens" }
                            TabButton { text: "Windows" }
                        }
                        ListView {
                            id: nativeShareList
                            Layout.fillWidth: true
                            Layout.preferredHeight: 210
                            clip: true
                            model: nativeShareTab.currentIndex === 0 ? nativeSharePicker.sources.screens
                                                                     : nativeSharePicker.sources.windows
                            delegate: NavigationButton {
                                required property int index
                                required property string modelData
                                width: nativeShareList.width
                                text: modelData
                                onClicked: {
                                    chat.startNativeShare(index, nativeShareTab.currentIndex === 1)
                                    nativeSharePicker.close()
                                }
                            }
                            ScrollBar.vertical: ScrollBar {}
                            Label {
                                anchors.centerIn: parent
                                visible: nativeShareList.count === 0
                                text: "No sources available"
                            }
                        }
                    }
                }

                Dialog {
                    id: nativeAudioSettings
                    parent: Overlay.overlay
                    width: Math.min(window.width - 32, 380)
                    title: "Call audio devices"
                    modal: true
                    anchors.centerIn: Overlay.overlay
                    standardButtons: Dialog.Close

                    ColumnLayout {
                        Label { text: "Microphone" }
                        ComboBox {
                            Layout.fillWidth: true
                            model: chat.nativeInputs
                            textRole: "name"
                            valueRole: "id"
                            currentIndex: -1
                            onActivated: chat.setNativeDevice(true, currentValue)
                        }
                        Label { text: "Speakers" }
                        ComboBox {
                            Layout.fillWidth: true
                            model: chat.nativeOutputs
                            textRole: "name"
                            valueRole: "id"
                            currentIndex: -1
                            onActivated: chat.setNativeDevice(false, currentValue)
                        }
                    }
                }

                Button {
                    visible: !chat.verified && !window.showCall
                    text: "Verify this device in User settings"
                    Layout.leftMargin: 20
                    Layout.topMargin: 8
                    onClicked: settings.open()
                }
                Label {
                    visible: text.length > 0
                    text: chat.status
                    color: window.muted
                    Layout.fillWidth: true
                    Layout.leftMargin: 20
                    wrapMode: Text.WordWrap
                }
                ListView {
                    id: timeline
                    visible: !window.showCall
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 4
                    onHeightChanged: Qt.callLater(alignShortTimeline)
                    onContentHeightChanged: Qt.callLater(alignShortTimeline)

                    function alignShortTimeline(stickToEnd = true) {
                        const margin = Math.max(0, height - contentHeight)
                        if (Math.abs(topMargin - margin) < 1)
                            return
                        const wasShort = topMargin > 0
                        topMargin = margin
                        if (stickToEnd && (wasShort || margin > 0))
                            positionViewAtEnd()
                    }
                    property string displayedRoom: ""
                    property string historyAnchor: ""
                    property var eventIndices: ({})
                    property string highlightedEvent: ""

                    function jumpToEvent(eventId) {
                        const index = eventIndices[eventId]
                        if (index === undefined)
                            return
                        positionViewAtIndex(index, ListView.Center)
                        highlightedEvent = eventId
                        replyHighlight.restart()
                    }

                    Timer {
                        id: replyHighlight
                        interval: 1600
                        onTriggered: timeline.highlightedEvent = ""
                    }

                    function refreshMessages() {
                        const messages = chat.messages
                        const sameRoom = displayedRoom === chat.selected
                        const anchor = historyAnchor
                        historyAnchor = ""
                        if (sameRoom && JSON.stringify(model) === JSON.stringify(messages))
                            return

                        if (!sameRoom)
                            highlightedEvent = ""
                        displayedRoom = chat.selected
                        const indices = Object.create(null)
                        messages.forEach((message, index) => {
                            if (message.eventId)
                                indices[message.eventId] = index
                        })
                        eventIndices = indices
                        model = messages
                        forceLayout()
                        alignShortTimeline(false)
                        const anchorIndex = sameRoom && anchor !== ""
                            ? messages.findIndex(message => message.id === anchor) : -1
                        if (anchorIndex >= 0)
                            positionViewAtIndex(anchorIndex, ListView.Beginning)
                        else
                            positionViewAtEnd()
                    }

                    Connections {
                        target: chat
                        function onOlderMessagesLoading() {
                            const top = timeline.contentY
                            let index = timeline.indexAt(1, top)
                            if (index < 0)
                                index = timeline.indexAt(1, top + timeline.spacing + 1)
                            timeline.historyAnchor = timeline.model?.[index]?.id ?? ""
                        }
                        function onTimelineChanged() { timeline.refreshMessages() }
                    }
                    delegate: Item {
                        id: message
                        required property var modelData
                        width: timeline.width
                        height: messageColumn.implicitHeight + (modelData.startsGroup ? 10 : 0)
                        readonly property var original: modelData.replyTo
                            ? timeline.model?.[timeline.eventIndices[modelData.replyTo]] : null

                        HoverHandler { id: messageHover }
                        Rectangle {
                            anchors.fill: parent
                            color: timeline.highlightedEvent === modelData.eventId && modelData.eventId
                                ? window.activeSurface : window.hoverSurface
                            visible: messageHover.hovered
                                || (timeline.highlightedEvent !== "" && timeline.highlightedEvent === modelData.eventId)
                        }

                        ColumnLayout {
                            id: messageColumn
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            anchors.leftMargin: 18
                            anchors.rightMargin: 22
                            anchors.topMargin: modelData.startsGroup ? 10 : 0
                            spacing: 6

                            RowLayout {
                                visible: modelData.day.length > 0
                                Layout.fillWidth: true
                                Layout.bottomMargin: 6
                                spacing: 10
                                Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
                                Label { text: modelData.day; color: window.muted; font.pixelSize: 12 }
                                Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 10

                                Item {
                                    Layout.preferredWidth: 34
                                    Layout.preferredHeight: modelData.startsGroup ? 34 : 0
                                    Layout.alignment: Qt.AlignTop

                                    Avatar {
                                        anchors.fill: parent
                                        visible: modelData.startsGroup
                                        source: visible ? modelData.avatar ?? "" : ""
                                        name: modelData.name
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 3
                                    RowLayout {
                                        visible: modelData.startsGroup
                                        Layout.fillWidth: true
                                        spacing: 8
                                        Label {
                                            text: modelData.name
                                            textFormat: Text.PlainText
                                            font.weight: Font.DemiBold
                                            elide: Text.ElideRight
                                            Layout.maximumWidth: message.width / 2
                                            ToolTip.text: modelData.sender
                                            ToolTip.visible: senderHover.hovered
                                            HoverHandler { id: senderHover }
                                        }
                                        Label { text: modelData.time; color: window.muted; font.pixelSize: 12 }
                                        Item { Layout.fillWidth: true }
                                    }
                                    NavigationButton {
                                        visible: !!modelData.replyTo
                                        enabled: !!message.original
                                        Layout.fillWidth: true
                                        text: message.original
                                            ? message.original.name + " · " + message.original.body.replace(/\s+/g, " ")
                                            : "Original message not loaded"
                                        Accessible.name: "In reply to " + text
                                        onClicked: timeline.jumpToEvent(modelData.replyTo)
                                    }
                                    Loader {
                                        active: !!modelData.hasImage
                                        visible: active
                                        Layout.fillWidth: true
                                        Layout.maximumWidth: 360
                                        Layout.preferredHeight: modelData.imageWidth > 0 && modelData.imageHeight > 0
                                            ? Math.max(96, Math.min(380, width * modelData.imageHeight / modelData.imageWidth))
                                            : 230
                                        Layout.alignment: Qt.AlignLeft

                                        sourceComponent: Button {
                                            id: imagePreview
                                            readonly property url fullSource: modelData.image ?? ""
                                            readonly property url thumbnailSource: modelData.thumbnail ?? ""
                                            property bool thumbnailFailed: false
                                            onThumbnailSourceChanged: thumbnailFailed = false
                                            enabled: fullSource.toString() !== ""
                                            Accessible.name: "Open image"
                                            onClicked: {
                                                imageViewer.imageSource = fullSource
                                                imageViewer.open()
                                            }
                                            background: Rectangle {
                                                radius: 8
                                                color: window.fieldSurface
                                                border.color: imagePreview.visualFocus ? window.accent : window.divider
                                            }
                                            contentItem: Item {
                                                Image {
                                                    id: thumbnailImage
                                                    anchors.fill: parent
                                                    source: !imagePreview.thumbnailFailed && imagePreview.thumbnailSource.toString() !== ""
                                                        ? imagePreview.thumbnailSource : imagePreview.fullSource
                                                    asynchronous: true
                                                    fillMode: Image.PreserveAspectFit
                                                    sourceSize.width: 720
                                                    sourceSize.height: 460
                                                    onStatusChanged: {
                                                        if (status === Image.Error && source.toString() === imagePreview.thumbnailSource.toString())
                                                            imagePreview.thumbnailFailed = true
                                                    }
                                                }
                                                Label {
                                                    anchors.centerIn: parent
                                                    visible: thumbnailImage.status !== Image.Ready
                                                    color: window.muted
                                                    text: thumbnailImage.status === Image.Loading ? "Loading image…" : "Image unavailable"
                                                }
                                            }
                                        }
                                    }
                                    RowLayout {
                                        Layout.fillWidth: true
                                        Label {
                                            text: modelData.body
                                            textFormat: Text.PlainText
                                            wrapMode: Text.Wrap
                                            Layout.fillWidth: true
                                            Layout.alignment: Qt.AlignTop
                                            font.pixelSize: 15
                                            Accessible.name: modelData.name + ", " + modelData.time + ": " + text
                                        }
                                        Rectangle {
                                            id: messageActions
                                            Layout.preferredWidth: actionButtons.implicitWidth + 8
                                            Layout.preferredHeight: actionButtons.implicitHeight + 4
                                            Layout.alignment: Qt.AlignTop
                                            radius: 7
                                            color: window.fieldSurface
                                            border.color: window.divider
                                            opacity: messageHover.hovered || replyAction.activeFocus
                                                || copyAction.activeFocus ? 1 : 0

                                            RowLayout {
                                                id: actionButtons
                                                anchors.centerIn: parent
                                                spacing: 2

                                                ToolButton {
                                                    id: replyAction
                                                    visible: !!modelData.canReply
                                                    enabled: chat.ready && !chat.loggingOut && !window.sendingImage
                                                    text: "↩"
                                                    Accessible.name: "Reply to " + modelData.name
                                                    ToolTip.text: "Reply"
                                                    ToolTip.visible: messageActions.opacity > 0 && hovered
                                                    Layout.preferredWidth: 30
                                                    Layout.preferredHeight: 24
                                                    onClicked: window.startReply(modelData)
                                                }
                                                ToolButton {
                                                    id: copyAction
                                                    text: "Copy"
                                                    Accessible.name: "Copy message text"
                                                    Layout.preferredHeight: 24
                                                    onClicked: chat.copyText(modelData.body)
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ScrollBar.vertical: ScrollBar {}
                }
                RowLayout {
                    visible: window.replyTarget !== null && !window.showCall
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    Layout.topMargin: 8
                    Label {
                        text: window.replyTarget
                            ? "Replying to " + window.replyTarget.name + " · " + window.replyTarget.body.replace(/\s+/g, " ")
                            : ""
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        color: window.muted
                        Layout.fillWidth: true
                    }
                    ToolButton {
                        text: "×"
                        Accessible.name: "Cancel reply"
                        enabled: !window.sendingImage
                        onClicked: window.replyTarget = null
                    }
                }
                Shortcut {
                    sequence: "Escape"
                    enabled: window.replyTarget !== null && !window.sendingImage && !settings.opened && !switcher.opened && !imageViewer.opened
                    onActivated: window.replyTarget = null
                }
                RowLayout {
                    visible: window.imageAttachment.toString() !== "" && !window.showCall
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    Layout.topMargin: 8
                    spacing: 10
                    Image {
                        source: visible ? window.imageAttachment : ""
                        asynchronous: true
                        fillMode: Image.PreserveAspectFit
                        sourceSize.width: 160
                        sourceSize.height: 120
                        Layout.preferredWidth: 80
                        Layout.preferredHeight: 60
                    }
                    Label {
                        text: window.sendingImage ? "Sending image…" : "Image attached"
                        Layout.fillWidth: true
                        color: window.muted
                    }
                    ToolButton {
                        text: "×"
                        Accessible.name: "Remove attached image"
                        enabled: !window.sendingImage
                        onClicked: window.imageAttachment = ""
                    }
                }
                Rectangle {
                    visible: chat.selected !== "" && !window.showCall
                    Layout.fillWidth: true
                    Layout.leftMargin: 10
                    Layout.rightMargin: 10
                    Layout.topMargin: 8
                    Layout.bottomMargin: 10
                    implicitHeight: composerControls.implicitHeight + 10
                    radius: 12
                    color: window.fieldSurface
                    border.color: composer.activeFocus ? window.accent : window.divider

                    RowLayout {
                        id: composerControls
                        anchors.fill: parent
                        anchors.margins: 5
                        spacing: 6
                        ToolButton {
                            text: "+"
                            Accessible.name: "Attach an image"
                            ToolTip.text: Accessible.name
                            ToolTip.visible: hovered
                            enabled: window.canAttachImage
                            onClicked: {
                                attachmentPicker.room = chat.selected
                                attachmentPicker.open()
                            }
                        }
                        ScrollView {
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(44, Math.min(168, composer.implicitHeight))
                            contentWidth: availableWidth
                            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                            ScrollBar.vertical.policy: ScrollBar.AsNeeded

                            TextArea {
                                id: composer
                                leftPadding: 6
                                rightPadding: 16
                                topPadding: 10
                                bottomPadding: 10
                                wrapMode: TextEdit.Wrap
                                textFormat: TextEdit.PlainText
                                background: Item {}
                                placeholderText: window.imageAttachment.toString() !== "" ? "Add a caption"
                                    : chat.roomName ? "Message " + chat.roomName : "Write a message"
                                Accessible.name: placeholderText
                                enabled: chat.ready && !chat.loggingOut && !window.sendingImage

                                DropArea {
                                    anchors.fill: parent
                                    enabled: window.canAttachImage
                                    onEntered: drag => {
                                        drag.accepted = drag.hasUrls && drag.urls.some(url => chat.canAttachImage(url))
                                    }
                                    onDropped: drop => {
                                        const image = drop.urls.find(url => chat.canAttachImage(url))
                                        if (image) {
                                            window.imageAttachment = image
                                            drop.acceptProposedAction()
                                            composer.forceActiveFocus()
                                        }
                                    }
                                }

                                Keys.onPressed: event => {
                                    if (!composer.inputMethodComposing && window.canAttachImage
                                            && event.matches(StandardKey.Paste)) {
                                        const image = chat.clipboardImage()
                                        if (image.toString() !== "") {
                                            window.imageAttachment = image
                                            event.accepted = true
                                            return
                                        }
                                    }
                                    const enter = event.key === Qt.Key_Return || event.key === Qt.Key_Enter
                                    if (!enter || composer.inputMethodComposing
                                            || (event.modifiers & ~Qt.KeypadModifier) !== Qt.NoModifier)
                                        return

                                    event.accepted = true
                                    if (!event.isAutoRepeat && sendButton.enabled)
                                        sendButton.clicked()
                                }
                            }
                        }
                        Button {
                            id: sendButton
                            text: window.sendingImage ? "Sending…" : "Send"
                            background: Rectangle {
                                radius: 7
                                color: sendButton.enabled ? window.activeSurface : "transparent"
                                border.color: sendButton.visualFocus ? window.accent : "transparent"
                            }
                            enabled: chat.ready && !chat.loggingOut && !window.sendingImage
                                && (window.imageAttachment.toString() !== "" ? chat.imageUploadRoom === ""
                                    : composer.text.trim().length > 0)
                            onClicked: {
                                const replyTo = window.replyTarget?.eventId ?? ""
                                if (window.imageAttachment.toString() !== "") {
                                    chat.sendImage(window.imageAttachment, composer.text, replyTo)
                                } else if (chat.send(composer.text, replyTo)) {
                                    composer.clear()
                                    window.replyTarget = null
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
