#include "deliver_pane.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "paperwork_view.h"
#include "render_queue_view.h"
#include "tray_shell.h"

#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontMetricsF>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QTimer>
#include <QVBoxLayout>

namespace kerf {

namespace {

using shell::text;

constexpr int kHeader = 44;

// A row's value: right-aligned in its style, elided at the right when the
// row is too narrow for it.
class Value final : public QWidget {
public:
    Value(const QString &text, const tokens::TypeStyle &style) : m_text(text), m_style(style) {
        setObjectName(QStringLiteral("deliverValue"));
        setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
        setMinimumWidth(0);
    }
    QSize sizeHint() const override { return {ui::snapWidth(m_style, m_text), m_style.line}; }
    QSize minimumSizeHint() const override { return {0, m_style.line}; }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        const QFont font = tokens::font(m_style);
        p.setFont(font);
        p.setPen(tokens::color(tokens::ink));
        p.drawText(rect(), Qt::AlignRight | Qt::AlignVCenter, QFontMetricsF(font).elidedText(m_text, Qt::ElideRight, width()));
    }

private:
    QString m_text;
    tokens::TypeStyle m_style;
};

QJsonArray kv(const QString &label, const QString &value, bool words = false) {
    QJsonArray row{label, value};
    if (words) row.append(QStringLiteral("ui"));
    return row;
}

} // namespace

DeliverPane::DeliverPane(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("deliverPane"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    // .clipb: 44 tall, pad 4 8 4 12.
    auto *header = new QWidget;
    header->setFixedHeight(kHeader);
    auto *headerLayout = new QVBoxLayout(header);
    headerLayout->setContentsMargins(tokens::space::s12, tokens::space::s4, tokens::space::s8, tokens::space::s4);
    headerLayout->setSpacing(0);
    m_title = text(QStringLiteral("Master"), tokens::title, tokens::ink);
    m_title->setObjectName(QStringLiteral("deliverTitle"));
    m_file = text(QStringLiteral("No master output yet"), tokens::monoSmall, tokens::ink3);
    m_file->setObjectName(QStringLiteral("deliverFile"));
    headerLayout->addWidget(m_title);
    headerLayout->addWidget(m_file);
    headerLayout->addStretch(1);
    layout->addWidget(header);
    m_body = new QWidget;
    m_bodyLayout = new QVBoxLayout(m_body);
    m_bodyLayout->setContentsMargins(0, 0, 0, 0);
    m_bodyLayout->setSpacing(0);
    layout->addWidget(m_body);
    // The destination's chooser, under the Destination rows.
    auto *chooser = new QHBoxLayout;
    chooser->setContentsMargins(tokens::space::s12, tokens::space::s4, tokens::space::s4, 0);
    chooser->addStretch(1);
    m_choose = new ui::GhostButton(QStringLiteral("Choose folder…"));
    m_choose->setObjectName(QStringLiteral("deliverChooseFolder"));
    m_choose->setControlHeight(tokens::height::h20);
    m_choose->setTypeStyle(tokens::meta);
    connect(m_choose, &QAbstractButton::clicked, this, &DeliverPane::chooseFolder);
    chooser->addWidget(m_choose);
    layout->addLayout(chooser);
    layout->addStretch(1);
    if (m_bus) {
        connect(m_bus, &EditorBus::snapshotChanged, this, &DeliverPane::refresh);
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &) {
            if (type == QStringLiteral("render_queue") || type == QStringLiteral("loudness") || type == QStringLiteral("export"))
                refresh();
        });
        connect(m_bus, &EditorBus::highlightPane, this, [this](const QString &id) {
            if (id == QStringLiteral("deliver")) highlight();
        });
    }
    connect(deliver::Settings::instance(), &deliver::Settings::changed, this, [this](const QString &directory) {
        if (deliver::projectName(directory) == deliver::projectName(projectDirectory())) refresh();
    });
    refresh();
}

void DeliverPane::setProjectDirectory(const QString &directory) {
    m_directory = directory;
    refresh();
}

QString DeliverPane::projectDirectory() const {
    return deliver::projectDirectory(this, m_directory);
}

void DeliverPane::refresh() {
    const QString directory = projectDirectory();
    const deliver::Output master = deliver::output(directory, QStringLiteral("master"));
    const QJsonObject timeline = m_bus ? m_bus->timeline() : QJsonObject();
    const QJsonObject sequence = deliver::masterSequence(timeline);
    // The configured folder is where the next render goes. The file is the
    // last completed master's while it went there (a failed or cancelled
    // master made nothing), else the planned name.
    const QJsonArray jobs =
        m_bus ? m_bus->lastEvent(QStringLiteral("render_queue")).value(QStringLiteral("jobs")).toArray() : QJsonArray();
    const QString file = deliver::shownFile(directory, master, deliver::lastCompleted(jobs, QStringLiteral("master")));
    QString folder = master.folder;
    if (folder.isEmpty()) folder = QStringLiteral("Project folder");

    const bool prores = master.codec == QStringLiteral("prores_hq");
    QJsonArray video{kv(QStringLiteral("Format"), prores ? QStringLiteral("QuickTime") : QStringLiteral("MPEG-4"), true),
                     kv(QStringLiteral("Codec"), deliver::codecName(master.quality, master.codec), true)};
    if (const QString size = deliver::sizeText(sequence); !size.isEmpty()) video.append(kv(QStringLiteral("Resolution"), size));
    const double fps = sequence.value(QStringLiteral("fps")).toDouble(m_bus ? m_bus->fps() : 0);
    if (const QString rate = deliver::rateText(fps); !rate.isEmpty()) video.append(kv(QStringLiteral("Frame rate"), rate));
    // The master is converted to Rec.709 and tagged so; other encodings
    // carry no tags, so nothing is claimed for them.
    if (prores) video.append(kv(QStringLiteral("Colour space"), QStringLiteral("Rec.709"), true));

    QJsonArray audio;
    if (prores) audio.append(kv(QStringLiteral("Format"), deliver::pcmText(timeline), true));
    // R128's target, as the loudness measurement and QC use it.
    const QJsonObject loudness = m_bus ? m_bus->lastEvent(QStringLiteral("loudness")) : QJsonObject();
    const double target = loudness.value(QStringLiteral("target")).toDouble(-23);
    audio.append(kv(QStringLiteral("Channels"),
                    QStringLiteral("Stereo · R128 %1 LUFS").arg(QString::number(target, 'g', 3).replace(QLatin1Char('-'), QChar(0x2212))),
                    true));

    bool captions = false;
    for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray())
        if (track.toObject().value(QStringLiteral("type")).toString() == QStringLiteral("subtitle")) captions = true;
    const QJsonArray destination{kv(QStringLiteral("Folder"), folder),
                                 kv(QStringLiteral("Paperwork"), PaperworkView::summaryFor(directory, captions), true)};

    m_choose->setEnabled(!directory.isEmpty());
    ui::setTip(m_choose, directory.isEmpty() ? QStringLiteral("Open a project to choose where its master goes")
                                             : QStringLiteral("Choose where the master goes"));

    setOutput(QJsonObject{
        {QStringLiteral("title"), master.title.isEmpty() ? QStringLiteral("Master") : master.title},
        {QStringLiteral("file"), file.isEmpty() ? QStringLiteral("No master output yet") : file},
        {QStringLiteral("groups"),
         QJsonArray{QJsonObject{{QStringLiteral("title"), QStringLiteral("Video")}, {QStringLiteral("rows"), video}},
                    QJsonObject{{QStringLiteral("title"), QStringLiteral("Audio")}, {QStringLiteral("rows"), audio}},
                    QJsonObject{{QStringLiteral("title"), QStringLiteral("Destination")}, {QStringLiteral("rows"), destination}}}}});
}

void DeliverPane::setOutput(const QJsonObject &output) {
    if (output == m_output && m_bodyLayout->count() > 0) return;
    m_output = output;
    m_title->setText(output.value(QStringLiteral("title")).toString(QStringLiteral("Master")));
    m_file->setText(output.value(QStringLiteral("file")).toString(QStringLiteral("No master output yet")));
    while (QLayoutItem *item = m_bodyLayout->takeAt(0)) {
        delete item->widget();
        delete item;
    }
    for (const QJsonValue &groupValue : output.value(QStringLiteral("groups")).toArray()) {
        const QJsonObject group = groupValue.toObject();
        // .ish: 28 tall, pad 0 12, UI 500 12 ink, margin-top 4.
        m_bodyLayout->addSpacing(tokens::space::s4);
        QLabel *heading = text(group.value(QStringLiteral("title")).toString(), tokens::controlStrong, tokens::ink);
        heading->setObjectName(QStringLiteral("deliverGroup"));
        heading->setFixedHeight(tokens::height::h28);
        heading->setContentsMargins(tokens::space::s12, 0, tokens::space::s12, 0);
        m_bodyLayout->addWidget(heading);
        for (const QJsonValue &rowValue : group.value(QStringLiteral("rows")).toArray()) {
            const QJsonArray row = rowValue.toArray();
            // .kv: 24 tall, pad 0 12, label UI 12 ink2, value mono 12 ink (UI for words).
            auto *line = new QWidget;
            line->setObjectName(QStringLiteral("deliverRow"));
            line->setFixedHeight(tokens::height::h24);
            auto *rowLayout = new QHBoxLayout(line);
            rowLayout->setContentsMargins(tokens::space::s12, 0, tokens::space::s12, 0);
            rowLayout->setSpacing(tokens::space::s8);
            QLabel *label = text(row.at(0).toString(), tokens::control, tokens::ink2);
            label->setObjectName(QStringLiteral("deliverLabel"));
            const bool words = row.at(2).toString() == QStringLiteral("ui");
            auto *value = new Value(row.at(1).toString(), words ? tokens::control : tokens::timecode);
            rowLayout->addWidget(label);
            rowLayout->addWidget(value, 1);
            m_bodyLayout->addWidget(line);
        }
    }
}

QString DeliverPane::titleText() const {
    return m_title->text();
}

QString DeliverPane::fileText() const {
    return m_file->text();
}

QString DeliverPane::value(const QString &label) const {
    for (const QJsonValue &group : m_output.value(QStringLiteral("groups")).toArray())
        for (const QJsonValue &row : group.toObject().value(QStringLiteral("rows")).toArray())
            if (row.toArray().at(0).toString() == label) return row.toArray().at(1).toString();
    return {};
}

QString DeliverPane::value(const QString &groupTitle, const QString &label) const {
    for (const QJsonValue &group : m_output.value(QStringLiteral("groups")).toArray()) {
        if (group.toObject().value(QStringLiteral("title")).toString() != groupTitle) continue;
        for (const QJsonValue &row : group.toObject().value(QStringLiteral("rows")).toArray())
            if (row.toArray().at(0).toString() == label) return row.toArray().at(1).toString();
    }
    return {};
}

void DeliverPane::setFolder(const QString &folder) {
    const QString directory = projectDirectory();
    if (directory.isEmpty() || folder.isEmpty()) return;
    // Settings::changed refreshes the tray, its queue and this pane.
    deliver::setOutputFolder(directory, QStringLiteral("master"), deliver::folderSetting(directory, folder));
}

void DeliverPane::chooseFolder() {
    const QString directory = projectDirectory();
    if (directory.isEmpty()) return;
    const QString current = deliver::output(directory, QStringLiteral("master")).folder;
    const QString start = current.isEmpty() ? directory : QDir::isAbsolutePath(current) ? current : QDir(directory).filePath(current);
    const QString chosen = QFileDialog::getExistingDirectory(this, QStringLiteral("Choose where the master goes"),
                                                             QFileInfo::exists(start) ? start : directory);
    if (!chosen.isEmpty()) setFolder(chosen);
}

void DeliverPane::highlight() {
    m_highlighted = true;
    const int flash = ++m_flash;
    update();
    QTimer::singleShot(1600, this, [this, flash] {
        if (flash != m_flash) return;
        m_highlighted = false;
        update();
    });
}

void DeliverPane::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, kHeader - 1, width(), 1), tokens::color(tokens::line));
    if (m_highlighted) {
        painter.setRenderHint(QPainter::Antialiasing);
        ui::paintInsetLine(painter, QRectF(rect()), 0, tokens::color(tokens::accent));
    }
}

} // namespace kerf
