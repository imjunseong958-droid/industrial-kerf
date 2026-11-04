#pragma once

// The Inspector's Deliver pane (mockup-spec 5.2), shown in the Deliver
// workspace while Video is chosen: the master the Deliver tray renders.
//
//   Master / <the master's file name>            44 pt header, bottom line
//   Video        Format, Codec, Resolution, Frame rate, Colour space
//   Audio        Format (PCM rate and bit depth), Channels and the R128 target
//   Destination  Folder (with Choose folder…), Paperwork
//
// Every value is the master output's real setting: the codec and container
// of its encoding, the size and rate of the sequence it renders, the
// colour space the master is tagged with, the timeline's sample rate, the
// project's folder (deliver/<project>/outputs, which export takes as its
// output) and the paperwork chosen in the tray. The file is the last
// master's, else the one the next render will be named.
//
// setOutput takes {title, file, groups:[{title, rows:[[label, value,
// "ui"?]]}]} (a third "ui" sets the value in the UI font, as words); the
// pane builds that itself from the bus and the project's settings.

#include <QJsonObject>
#include <QString>
#include <QWidget>

class QLabel;
class QVBoxLayout;

namespace kerf {

class EditorBus;

namespace ui {
class GhostButton;
} // namespace ui

class DeliverPane final : public QWidget {
    Q_OBJECT
public:
    explicit DeliverPane(QWidget *parent, EditorBus *bus);
    void setProjectDirectory(const QString &directory);
    QString projectDirectory() const;
    void setOutput(const QJsonObject &output);
    QJsonObject output() const { return m_output; }
    // Build the output from the bus and the project's settings again.
    void refresh();
    // The header's two lines: "Master", "atlas_master.mov".
    QString titleText() const;
    QString fileText() const;
    // "Codec" → "ProRes 422 HQ", from the rows shown (the first of that label).
    QString value(const QString &label) const;
    // The value of `label` in the group `group` ("Audio", "Format").
    QString value(const QString &group, const QString &label) const;
    // Keep `folder` (a folder on disk) as the master's destination.
    void setFolder(const QString &folder);
    // Choose the destination folder in a dialog.
    void chooseFolder();
    ui::GhostButton *folderButton() const { return m_choose; }
    // Flash the pane's outline (the bus's highlightPane "deliver").
    void highlight();
    bool highlighted() const { return m_highlighted; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    EditorBus *m_bus = nullptr;
    QString m_directory;
    QJsonObject m_output;
    QLabel *m_title = nullptr;
    QLabel *m_file = nullptr;
    QWidget *m_body = nullptr;
    QVBoxLayout *m_bodyLayout = nullptr;
    ui::GhostButton *m_choose = nullptr;
    bool m_highlighted = false;
    int m_flash = 0;
};

} // namespace kerf
