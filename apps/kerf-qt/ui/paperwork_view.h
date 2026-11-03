#pragma once

// The Deliver tray's Paperwork pane (mockup-spec 13): what travels with the
// master. 32-point rows, each a checkbox and its name (UI 12 ink) with its
// file or count on the right (mono 11 ink3):
//
//   Sequence markers     N notes              export_comments {format: markers}
//   Comments as CSV      <name>_notes.csv     export_comments {format: csv, path}
//   EDL locators         <name>_locators.edl  export_comments {format: edl, path}
//   Include resolved notes                    includeResolved on the three above
//   QC report            <name>_qc.pdf        written here (QPdfWriter) from the last check
//   Captions as SRT      <name>_captions.srt  export_captions {format: srt, path};
//                                             only while a subtitle track exists
//   Copy review link     kerf://<project>/rev<N>
//
// The choices are kept per project (deliver/<project>/<option>; the EDL
// one, edlLocators, is the toolbar Export menu's too). When the master
// finishes rendering, deliver(masterPath) runs the chosen items with their
// files beside the master, and each row then says how its item went.

#include <QHash>
#include <QJsonObject>
#include <QList>
#include <QString>
#include <QStringList>
#include <QWidget>

#include <functional>

namespace kerf {

class EditorBus;

class PaperworkView final : public QWidget {
    Q_OBJECT
public:
    explicit PaperworkView(QWidget *parent, EditorBus *bus);

    void setProjectDirectory(const QString &directory);
    QString projectDirectory() const;

    // Row ids: markers, csv, edlLocators, includeResolved, qcReport,
    // captions, link.
    QStringList rowIds() const;
    QWidget *row(const QString &id) const;
    bool isOn(const QString &id) const;
    void setOn(const QString &id, bool on);
    // The right side of a row: its file, count, link or how it went.
    QString detailText(const QString &id) const;
    // How the row's item went in the last delivery: "", "Writing…",
    // "Written", or what went wrong.
    QString statusText(const QString &id) const;
    bool statusIsBad(const QString &id) const;
    // "CSV · EDL · QC report": the chosen files that travel beside the
    // master (markers stay in the sequence).
    QString summaryText() const;
    // The same for a project's saved choices, for the Deliver pane.
    static QString summaryFor(const QString &directory, bool captions);
    static QString summaryOf(const std::function<bool(const QString &)> &on, bool captions);
    // A subtitle track exists, so captions can travel too.
    bool hasCaptions() const;

    // kerf://<project>/rev<N>, with ?branch=…&head=…&remote=… when the
    // collab status names them; empty without a project or a revision.
    QString reviewLink() const;
    // The link as the row shows it: kerf://<project>/rev<N>.
    QString reviewLinkText() const;
    // Put the review link on the clipboard; false when there is none.
    bool copyReviewLink();

    // Run the chosen items for the master at `masterPath`, their files
    // beside it.
    void deliver(const QString &masterPath);
    // The file names an item writes for the master at `masterPath` (or the
    // planned master when empty): "atlas_notes.csv".
    QString fileFor(const QString &id, const QString &masterPath = {}) const;

    // The QC report: the check's rows and findings, the programme loudness,
    // the project, sequence and revision, as a PDF at `path` (written
    // atomically). `info` carries {project, sequence, revision, master,
    // fps, created}. False with `error` set when it could not be written.
    static bool writeQcReport(const QString &path, const QJsonObject &check, const QJsonObject &loudness,
                              const QJsonObject &info, QString *error = nullptr);

signals:
    // A choice changed.
    void optionsChanged();

protected:
    void resizeEvent(QResizeEvent *event) override;

private:
    class OptionRow;
    struct Pending {
        QString id;
        QString command;
        QString format;
        QString path;
    };
    void rebuild();
    void layoutRows();
    void handleEvent(const QString &type, const QJsonObject &event);
    void setStatus(const QString &id, const QString &text, bool bad);
    QString baseName(const QString &masterPath) const;
    int noteCount() const;
    QJsonObject reportInfo(const QString &masterPath) const;

    EditorBus *m_bus = nullptr;
    QString m_directory;
    QList<OptionRow *> m_rows;
    QHash<QString, QString> m_status;
    QHash<QString, bool> m_bad;
    // In-memory choices for a window without a project.
    QHash<QString, bool> m_unsaved;
    QList<Pending> m_pending;
    QString m_lastMaster;
    // The last check that finished, for the QC report.
    QJsonObject m_check;
};

} // namespace kerf
