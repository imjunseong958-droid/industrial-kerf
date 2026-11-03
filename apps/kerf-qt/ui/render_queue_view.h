#pragma once

// The Deliver tray's Masters pane (mockup-spec 13): the project's outputs,
// each a 52-point row with its title, the spec line its real settings make
// and, on the right, its status or its progress and percentage; then the
// path row, the master's file name and where it goes.
//
// The outputs are configured per project (QSettings
// deliver/<project>/outputs): Master (ProRes 422 HQ in QuickTime, 24-bit
// PCM), Draft (the 12 Mb/s H.264 review copy), Render preview (the
// program's preview render, from `preview` events) and Vertical cut (only
// while the project has a portrait sequence besides the master's; it
// renders that sequence). The bridge's `render_queue` and `export` events
// (plan section 3) give each output its latest job; a job no output names
// gets a row of its own while it is queued or running (and the latest such
// job once it has ended), so kerfd's job history of earlier sessions does
// not fill the pane. The rows scroll when they do not fit over the path
// row, which stays under them. A running or queued job is cancelled with
// export_cancel {jobId}.
//
// kerf::deliver holds what the tray, this view, the paperwork and the
// Inspector's Deliver pane share: the project's settings keys, the output
// configuration and the words for a sequence's settings.

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QPointer>
#include <QSet>
#include <QString>
#include <QStringList>
#include <QWidget>

class QAction;

namespace kerf {

class EditorBus;

namespace ui {
class FadeScrollArea;
class GhostButton;
class IconButton;
} // namespace ui

namespace deliver {

// The project folder a widget delivers: `explicitDirectory` when set,
// else the folder of its window's file (MainWindow sets windowFilePath to
// <project>/project.kerf); empty when there is none.
QString projectDirectory(const QWidget *widget, const QString &explicitDirectory);
// The project's folder name ("atlas"): the slug of settings keys and
// kerf:// links; empty without a project.
QString projectName(const QString &directory);
// The project's name as the document gives it (timeline.name, "Atlas"),
// else its folder's name.
QString projectTitle(const QString &directory, const QJsonObject &timeline);
// The QSettings key of `key` for the project: deliver/<project name>/<key>.
// The toolbar's Export menu reads `edlLocators` from here too. Empty
// without a project, which keeps nothing.
QString settingsKey(const QString &directory, const QString &key);

// A paperwork option of the project ("markers", "csv", "edlLocators",
// "includeResolved", "qcReport", "captions") and its default.
bool option(const QString &directory, const QString &key);
bool defaultOption(const QString &key);
void setOption(const QString &directory, const QString &key, bool on);

// One configured output.
struct Output {
    QString id;      // master, draft, preview, vertical
    QString title;   // Master, Draft, Render preview, Vertical cut
    QString quality; // master, review, full; empty for the preview
    QString codec;   // prores_hq, h264, hevc; empty for the preview
    QString folder;  // where the file goes (relative to the project or absolute); empty for the project folder
    QString name;    // the file's name without its extension; empty lets kerfd name it
};

// The outputs of the project with their saved folders and names (the
// defaults: deliveries/, and <project>_master, _draft, _vertical).
QList<Output> outputs(const QString &directory);
Output output(const QString &directory, const QString &id);
// Keep `folder` as the output's (all outputs of a project share the master's
// folder unless one was set for them).
void setOutputFolder(const QString &directory, const QString &id, const QString &folder);
// A folder chosen on disk as the output config keeps it: relative to the
// project ("deliveries/atlas/") when inside it, else absolute.
QString folderSetting(const QString &directory, const QString &chosen);
// The file's extension the output's encoding makes: mov for ProRes, else mp4.
QString extension(const Output &output);
// "atlas_master.mov", or empty when the output names no file.
QString plannedFile(const Output &output);
// The folder the output renders to, absolute: its folder against the
// project's (the project's own when it names none). Empty without a project
// for a relative folder.
QString folderPath(const QString &directory, const Output &output);
// The file the output's next render makes, as far as is known: the name of
// `completed` (the output's last completed job) while that file is in the
// output's configured folder, else the planned name. A folder chosen since
// that render makes the planned name come back.
QString shownFile(const QString &directory, const Output &output, const QJsonObject &completed);
// The last job of `jobs` (in order) with quality `quality` and status
// completed; {} when none.
QJsonObject lastCompleted(const QJsonArray &jobs, const QString &quality);

// The export command that renders `output` (of `sequence` when not empty):
// {type: export, quality, codec? (left out when it is the quality's own),
// sequence?, output?, name?}.
QJsonObject exportCommand(const Output &output, const QString &sequence = {});

// Fired whenever a project's delivery settings change, so the tray, its
// panes and the Inspector's Deliver pane show the same.
class Settings final : public QObject {
    Q_OBJECT
public:
    static Settings *instance();
    void notify(const QString &directory) { emit changed(directory); }
signals:
    void changed(const QString &directory);
};

// The program's sequences, from the timeline (timeline.sequences, else the
// timeline itself as one sequence): {id, name, width, height, fps, duration}.
QJsonArray sequences(const QJsonObject &timeline);
// The sequence a render without a sequence makes: the first.
QJsonObject masterSequence(const QJsonObject &timeline);
// A portrait sequence (taller than wide) other than the master's, or empty.
QJsonObject portraitSequence(const QJsonObject &timeline);
// "25p", "29.97p"; empty for no rate.
QString rateText(double fps);
// "1920×1080"; empty without a size.
QString sizeText(const QJsonObject &sequence);
// "ProRes 422 HQ", "H.264", "HEVC".
QString codecName(const QString &quality, const QString &codec);
// "PCM 48 kHz 24-bit" (the master's sound), from the timeline's sampleRate.
QString pcmText(const QJsonObject &timeline);

} // namespace deliver

class RenderQueueView final : public QWidget {
    Q_OBJECT
public:
    explicit RenderQueueView(QWidget *parent, EditorBus *bus);

    void setProjectDirectory(const QString &directory);
    QString projectDirectory() const;
    // The registry action that says renders can run now (export.master):
    // Draft and Vertical cut can be started while it is enabled.
    void setRenderAction(QAction *action);
    // Open blocking notes (-1 unknown): the Master row says them while it
    // has no job.
    void setBlocking(int blocking);

    // A `render_queue` event: {jobs:[{jobId, name, quality, codec,
    // sequence, status, progress, outputPath, error?}]}.
    void setQueue(const QJsonObject &queue);
    // An `export` event of a queued job: {status, jobId, name, path, progress}.
    void applyExport(const QJsonObject &event);
    // A `preview` event: the Render preview row's status.
    void setPreview(const QJsonObject &preview);
    // Rebuild from the bus's timeline and the project's settings.
    void refresh();

    // The rows shown, by id: the outputs', then "job:<jobId>" for jobs no
    // output names.
    QStringList rowIds() const;
    int rowCount() const { return int(m_rows.size()); }
    // Rows that show a job of the queue.
    int jobRowCount() const;
    QString titleText(const QString &id) const;
    QString specText(const QString &id) const;
    // The status on the right ("Ready", "Not started", "2 blocking notes
    // open"), or the percentage while it renders ("38%").
    QString statusText(const QString &id) const;
    // 0..100 while the row's job runs, else -1.
    int progress(const QString &id) const;
    bool statusIsBad(const QString &id) const;
    // The row's job ({} when none).
    QJsonObject job(const QString &id) const;
    // "atlas_master.mov → deliveries/": the master's file and its
    // configured folder, where the next render goes.
    QString pathText() const;
    // Where the path row is drawn, in this view's coordinates (under the
    // rows, which scroll when they do not all fit above it).
    QRect pathRowRect() const;
    // "3 outputs".
    QString summaryText() const;
    QWidget *row(const QString &id) const;
    // Render the output: export with its settings (Vertical cut renders
    // the portrait sequence). A master with nothing but the registry's
    // defaults to say runs `export.master` itself. The tray's Render
    // master calls this for "master" once nothing blocks it. False when it
    // was not sent.
    bool start(const QString &id);
    // Cancel the row's queued or running job (export_cancel).
    bool cancel(const QString &id);
    // The jobs as merged from render_queue and export events.
    QHash<QString, QJsonObject> jobs() const { return m_jobs; }

signals:
    // The rows changed (count or contents).
    void rowsChanged();
    // A job of `outputId` ("master", ...) was seen to finish rendering:
    // it went from queued or running to completed here.
    void jobCompleted(const QString &outputId, const QJsonObject &job);

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    struct RowState {
        QString id;
        QString title;
        QString spec;
        QString status;
        bool bad = false;
        bool quiet = false; // ink3 status ("Not started")
        int progress = -1;
        QJsonObject job;
    };
    class Row;
    // What a job's output is told by: read once per rebuild, not per job.
    struct Owners {
        QList<deliver::Output> outputs;
        QString portrait; // the portrait sequence's id
        QString first;    // the master sequence's id
    };
    void rebuild();
    Owners owners() const;
    QString outputOf(const QJsonObject &job, const Owners &owners) const;
    RowState stateOf(const deliver::Output &output, const Owners &owners) const;
    RowState stateOfJob(const QJsonObject &job) const;
    void layoutRows();
    void placeRows();

    EditorBus *m_bus = nullptr;
    QString m_directory;
    QPointer<QAction> m_renderAction;
    int m_blocking = -1;
    QJsonObject m_preview;
    // jobId → the job as last heard, in the order first heard.
    QHash<QString, QJsonObject> m_jobs;
    QStringList m_jobOrder;
    // jobId → the status last seen, for jobCompleted.
    QHash<QString, QString> m_seen;
    // jobIds heard queued or running here: a job no output names shows
    // while it is active, and the latest of these once it has ended.
    QSet<QString> m_ranHere;
    // output id → the render name sent, until its job is heard of.
    QHash<QString, QString> m_requested;
    ui::FadeScrollArea *m_scroll = nullptr;
    QWidget *m_rowsHost = nullptr;
    QList<Row *> m_rows;
    QString m_path;
    int m_pathTop = 0;
};

} // namespace kerf
