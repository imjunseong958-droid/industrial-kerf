#include "project_launcher.h"
#include "bridge.h"

#include <QCheckBox>
#include <QComboBox>
#include <QDateTime>
#include <QDir>
#include <QDoubleSpinBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QFormLayout>
#include <QFrame>
#include <QGridLayout>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMessageBox>
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QProgressBar>
#include <QPushButton>
#include <QRegularExpression>
#include <QScrollArea>
#include <QSettings>
#include <QSignalBlocker>
#include <QStackedWidget>
#include <QStandardPaths>
#include <QStyle>
#include <QTimer>
#include <QToolButton>
#include <QVBoxLayout>

#include <algorithm>
#include <utility>

namespace kerf {

namespace {

constexpr int kPathRole = Qt::UserRole;
constexpr int kExistsRole = Qt::UserRole + 1;
const QString kRecentKey = QStringLiteral("projects/recent");

QJsonObject callJson(char *(*function)(const char *), const QJsonObject &argument) {
    const QByteArray json = QJsonDocument(argument).toJson(QJsonDocument::Compact);
    char *raw = function(json.constData());
    if (!raw) return {{QStringLiteral("error"), QStringLiteral("Kerf did not answer.")}};
    const QJsonDocument document = QJsonDocument::fromJson(QByteArray(raw));
    kerf_bridge_string_free(raw);
    if (!document.isObject()) return {{QStringLiteral("error"), QStringLiteral("Kerf's answer could not be read.")}};
    return document.object();
}

void repolish(QWidget *widget) {
    widget->style()->unpolish(widget);
    widget->style()->polish(widget);
}

QString homeRelative(const QString &path) {
    const QString home = QDir::homePath();
    return path.startsWith(home + QLatin1Char('/')) ? QStringLiteral("~") + path.mid(home.size()) : path;
}

// A location as typed, with `~` for the home folder; empty stays empty.
QString expanded(const QString &text) {
    const QString trimmed = text.trimmed();
    if (trimmed == QStringLiteral("~")) return QDir::homePath();
    if (trimmed.startsWith(QStringLiteral("~/"))) return QDir::cleanPath(QDir::homePath() + trimmed.mid(1));
    return trimmed.isEmpty() ? QString() : QDir::cleanPath(trimmed);
}

// Addresses git clones from and pushes to: URLs, scp-style `user@host:path`,
// and local paths.
bool isGitAddress(const QString &address) {
    static const QRegularExpression url(QStringLiteral(R"(^(https?|ssh|git|file)://\S+$)"));
    static const QRegularExpression scp(QStringLiteral(R"(^[\w.-]+@[\w.-]+:\S+$)"));
    return url.match(address).hasMatch() || scp.match(address).hasMatch() || QDir::isAbsolutePath(address);
}

// The folder scaffold::folder_name makes for a project name.
QString folderName(const QString &name) {
    QString cleaned = name.trimmed();
    for (QChar &c : cleaned)
        if (c == QLatin1Char('/') || c == QLatin1Char('\\') || c == QLatin1Char(':') || c.isNull()) c = QLatin1Char('-');
    while (cleaned.startsWith(QLatin1Char('.'))) cleaned.remove(0, 1);
    while (cleaned.endsWith(QLatin1Char('.'))) cleaned.chop(1);
    return cleaned.trimmed();
}

// A folder a new project or clone cannot go into: a file, or a folder with
// anything in it.
bool occupied(const QString &path) {
    const QFileInfo info(path);
    if (!info.exists()) return false;
    return !info.isDir() ||
           !QDir(path).entryList(QDir::AllEntries | QDir::Hidden | QDir::System | QDir::NoDotAndDotDot).isEmpty();
}

QString defaultLocation() {
    const QString saved = QSettings().value(QStringLiteral("projects/location")).toString();
    if (!saved.isEmpty()) return saved;
    QString movies = QStandardPaths::writableLocation(QStandardPaths::MoviesLocation);
    if (movies.isEmpty()) movies = QDir::homePath();
    return QDir(movies).filePath(QStringLiteral("Kerf"));
}

QString untitledName(const QString &location) {
    QString name = QStringLiteral("Untitled");
    for (int n = 2; occupied(QDir(location).filePath(name)) && n < 1000; ++n)
        name = QStringLiteral("Untitled %1").arg(n);
    return name;
}

QPixmap canvas(const QSize &size) {
    QPixmap pixmap(size * 2);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    return pixmap;
}

// The same picture when selected: lists otherwise tint selected icons with
// the palette's highlight.
QIcon untinted(const QPixmap &pixmap) {
    QIcon icon(pixmap);
    icon.addPixmap(pixmap, QIcon::Selected);
    return icon;
}

// Sidebar symbols on a 16-point grid, drawn like the toolbar's.
QIcon sidebarIcon(int page) {
    QPixmap pixmap = canvas(QSize(16, 16));
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(QColor("#c4c4c4"), 1.3, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    QPainterPath path;
    switch (page) {
    case ProjectLauncher::NewProject:
        path.moveTo(8, 3.5); path.lineTo(8, 12.5);
        path.moveTo(3.5, 8); path.lineTo(12.5, 8);
        break;
    case ProjectLauncher::Recent:
        path.addEllipse(QPointF(8, 8), 5.5, 5.5);
        path.moveTo(8, 5); path.lineTo(8, 8); path.lineTo(10.5, 9.5);
        break;
    case ProjectLauncher::FromPreset:
        path.addRoundedRect(QRectF(2.5, 5.5, 11, 8), 1.5, 1.5);
        path.moveTo(4.5, 3.5); path.lineTo(11.5, 3.5);
        path.moveTo(6.5, 7.8); path.lineTo(6.5, 11.2); path.lineTo(9.8, 9.5); path.closeSubpath();
        break;
    case ProjectLauncher::Clone:
        path.moveTo(8, 2.5); path.lineTo(8, 10.5);
        path.moveTo(5, 7.5); path.lineTo(8, 10.5); path.lineTo(11, 7.5);
        path.moveTo(3, 13.5); path.lineTo(13, 13.5);
        break;
    }
    painter.drawPath(path);
    return untinted(pixmap);
}

// A template tile: the empty frame at its aspect over a short timeline.
QIcon templateTile(int width, int height) {
    const QSize tile(156, 100);
    QPixmap pixmap = canvas(tile);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(QColor("#303030"), 1));
    painter.setBrush(QColor("#1d1d1d"));
    painter.drawRoundedRect(QRectF(0.5, 0.5, tile.width() - 1, tile.height() - 1), 8, 8);
    const QRectF area(18, 10, tile.width() - 36, tile.height() - 36);
    QSizeF frame(width, height);
    frame.scale(area.size(), Qt::KeepAspectRatio);
    const QRectF picture(area.center().x() - frame.width() / 2, area.center().y() - frame.height() / 2,
                         frame.width(), frame.height());
    painter.setPen(QPen(QColor("#6a6a6a"), 1.2));
    painter.setBrush(QColor("#0e0e0e"));
    painter.drawRoundedRect(picture, 2, 2);
    const QPointF center = picture.center();
    painter.setPen(Qt::NoPen);
    painter.setBrush(QColor("#f0b429"));
    painter.drawPolygon(QPolygonF{{center.x() - 3.5, center.y() - 5}, {center.x() - 3.5, center.y() + 5},
                                  {center.x() + 5, center.y()}});
    const qreal lane = tile.height() - 19;
    painter.setBrush(QColor("#3b3b3b"));
    painter.drawRoundedRect(QRectF(18, lane, 50, 7), 2, 2);
    painter.setBrush(QColor("#4b4b4b"));
    painter.drawRoundedRect(QRectF(70, lane, 68, 7), 2, 2);
    painter.setPen(QPen(QColor("#f0b429"), 1.2));
    painter.drawLine(QPointF(58, lane - 4), QPointF(58, lane + 11));
    return untinted(pixmap);
}

// A project folder in the Recent list.
QIcon projectIcon(bool exists) {
    QPixmap pixmap = canvas(QSize(28, 20));
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(QColor(exists ? "#7a7a7a" : "#4a4a4a"), 1.1));
    painter.setBrush(QColor("#151515"));
    painter.drawRoundedRect(QRectF(1.5, 1.5, 25, 17), 3, 3);
    painter.setPen(Qt::NoPen);
    painter.setBrush(QColor(exists ? "#f0b429" : "#5a5a5a"));
    painter.drawRect(QRectF(5, 13, 10, 2.2));
    painter.setBrush(QColor(exists ? "#5a5a5a" : "#3a3a3a"));
    painter.drawRect(QRectF(16, 13, 7, 2.2));
    return untinted(pixmap);
}

QPushButton *chooseButton(const QString &objectName) {
    auto *button = new QPushButton(QStringLiteral("Choose…"));
    button->setObjectName(objectName);
    return button;
}

} // namespace

ProjectLauncher::ProjectLauncher(const QString &kerfdBinary, QWidget *parent) : QDialog(parent), m_kerfd(kerfdBinary) {
    setObjectName(QStringLiteral("projectLauncher"));
    setWindowTitle(QStringLiteral("Choose a Project"));
    setAttribute(Qt::WA_StyledBackground);
    resize(1000, 720);
    setMinimumSize(860, 620);
    m_inspectProject = [](const QJsonObject &request) { return callJson(kerf_project_inspect, request); };
    m_inspectPreset = [](const QJsonObject &request) { return callJson(kerf_preset_inspect, request); };

    auto *root = new QVBoxLayout(this);
    root->setContentsMargins(0, 0, 0, 0);
    root->setSpacing(0);
    auto *body = new QHBoxLayout;
    body->setContentsMargins(0, 0, 0, 0);
    body->setSpacing(0);
    root->addLayout(body, 1);

    m_sidebar = new QListWidget;
    m_sidebar->setObjectName(QStringLiteral("launcherSidebar"));
    m_sidebar->setFixedWidth(206);
    m_sidebar->setIconSize(QSize(16, 16));
    m_sidebar->setFrameShape(QFrame::NoFrame);
    for (const auto &[page, label] : {std::pair{NewProject, QStringLiteral("New Project")},
                                      std::pair{Recent, QStringLiteral("Recent")},
                                      std::pair{FromPreset, QStringLiteral("From Preset")},
                                      std::pair{Clone, QStringLiteral("Clone from GitHub")}})
        m_sidebar->addItem(new QListWidgetItem(sidebarIcon(page), label));
    body->addWidget(m_sidebar);

    auto *content = new QWidget;
    content->setObjectName(QStringLiteral("launcherContent"));
    content->setAttribute(Qt::WA_StyledBackground);
    auto *contentLayout = new QVBoxLayout(content);
    contentLayout->setContentsMargins(28, 22, 28, 12);
    contentLayout->setSpacing(4);
    m_heading = new QLabel;
    m_heading->setObjectName(QStringLiteral("launcherHeading"));
    contentLayout->addWidget(m_heading);
    m_subheading = new QLabel;
    m_subheading->setObjectName(QStringLiteral("launcherHint"));
    m_subheading->setWordWrap(true);
    contentLayout->addWidget(m_subheading);
    contentLayout->addSpacing(12);
    m_pages = new QStackedWidget;
    m_pages->setObjectName(QStringLiteral("launcherPages"));
    m_pages->addWidget(buildTemplates());
    m_pages->addWidget(buildRecent());
    m_pages->addWidget(buildPresetPage());
    m_pages->addWidget(buildClone());
    contentLayout->addWidget(m_pages, 1);
    contentLayout->addWidget(buildDetails());
    body->addWidget(content, 1);
    root->addWidget(buildFooter());

    connect(m_sidebar, &QListWidget::currentRowChanged, this, [this](int row) {
        if (row >= 0) showPage(static_cast<Page>(row));
    });
    reloadRecent();
    reloadPresetChoices();
    const int saved = QSettings().value(QStringLiteral("launcher/page"), int(NewProject)).toInt();
    showPage(saved >= NewProject && saved <= Clone ? static_cast<Page>(saved) : NewProject);

    // What can share a project: git, Git LFS, and a signed-in GitHub CLI.
    runTask(QJsonObject{{QStringLiteral("type"), QStringLiteral("tools")}}, [this](const QJsonObject &status) {
        const QString state = status.value(QStringLiteral("status")).toString();
        if (state == QStringLiteral("done")) {
            setTools(status);
        } else if (state != QStringLiteral("running")) {
            // Sharing still works when git is there; the task says if not.
            m_gitHint->setText(QStringLiteral("Kerf could not look for Git and the GitHub CLI: %1")
                                   .arg(status.value(QStringLiteral("error")).toString()));
        }
    });
}

ProjectLauncher::~ProjectLauncher() {
#ifndef KERF_QT_TESTING
    for (void *task : std::as_const(m_tasks)) kerf_task_free(task);
#endif
}

QWidget *ProjectLauncher::buildTemplates() {
    m_templates = new QListWidget;
    m_templates->setObjectName(QStringLiteral("templateTiles"));
    m_templates->setViewMode(QListView::IconMode);
    m_templates->setIconSize(QSize(156, 100));
    m_templates->setGridSize(QSize(188, 156));
    m_templates->setMovement(QListView::Static);
    m_templates->setResizeMode(QListView::Adjust);
    m_templates->setWordWrap(true);
    m_templates->setFrameShape(QFrame::NoFrame);
    struct Template {
        const char *id;
        const char *label;
        int width;
        int height;
    };
    for (const Template &entry : {Template{"standard", "Empty 16:9", 1920, 1080}, Template{"vertical", "Vertical 9:16", 1080, 1920},
                                  Template{"square", "Square 1:1", 1080, 1080}}) {
        auto *item = new QListWidgetItem(templateTile(entry.width, entry.height),
                                         QStringLiteral("%1\n%2 × %3").arg(QString::fromLatin1(entry.label)).arg(entry.width).arg(entry.height));
        item->setData(kPathRole, QString::fromLatin1(entry.id));
        item->setTextAlignment(Qt::AlignHCenter | Qt::AlignTop);
        m_templates->addItem(item);
    }
    const QString saved = QSettings().value(QStringLiteral("projects/template"), QStringLiteral("standard")).toString();
    for (int row = 0; row < m_templates->count(); ++row)
        if (m_templates->item(row)->data(kPathRole).toString() == saved) m_templates->setCurrentRow(row);
    if (!m_templates->currentItem()) m_templates->setCurrentRow(0);
    connect(m_templates, &QListWidget::currentItemChanged, this, [this] {
        updateDetailsSummary();
        updateChoose();
    });
    connect(m_templates, &QListWidget::itemActivated, this, [this] { choose(); });
    return m_templates;
}

QWidget *ProjectLauncher::buildRecent() {
    m_recent = new QListWidget;
    m_recent->setObjectName(QStringLiteral("recentProjects"));
    m_recent->setIconSize(QSize(28, 20));
    m_recent->setFrameShape(QFrame::NoFrame);
    m_recent->setTextElideMode(Qt::ElideMiddle);
    connect(m_recent, &QListWidget::currentItemChanged, this, [this] { updateChoose(); });
    connect(m_recent, &QListWidget::itemActivated, this, [this] { choose(); });
    return m_recent;
}

QWidget *ProjectLauncher::buildPresetPage() {
    auto *page = new QWidget;
    auto *layout = new QVBoxLayout(page);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(8);
    auto *pickRow = new QHBoxLayout;
    auto *libraryLabel = new QLabel(QStringLiteral("PRESETS IN YOUR PROJECTS"));
    libraryLabel->setObjectName(QStringLiteral("launcherEyebrow"));
    pickRow->addWidget(libraryLabel);
    pickRow->addStretch(1);
    auto *pickFile = new QPushButton(QStringLiteral("Choose a Preset File…"));
    pickFile->setObjectName(QStringLiteral("presetChooseFile"));
    connect(pickFile, &QPushButton::clicked, this, [this] {
        const QString path = QFileDialog::getOpenFileName(this, QStringLiteral("Choose a preset"), defaultLocation(),
                                                          QStringLiteral("Kerf editing preset (*.kerfpreset)"));
        if (!path.isEmpty()) loadPreset(path);
    });
    pickRow->addWidget(pickFile);
    layout->addLayout(pickRow);
    m_presetChoices = new QListWidget;
    m_presetChoices->setObjectName(QStringLiteral("presetChoices"));
    m_presetChoices->setMaximumHeight(96);
    m_presetChoices->setFrameShape(QFrame::NoFrame);
    connect(m_presetChoices, &QListWidget::currentItemChanged, this, [this](QListWidgetItem *item) {
        if (item && item->flags().testFlag(Qt::ItemIsEnabled)) loadPreset(item->data(kPathRole).toString());
    });
    layout->addWidget(m_presetChoices);
    m_presetTitle = new QLabel;
    m_presetTitle->setObjectName(QStringLiteral("launcherEyebrow"));
    m_presetTitle->setWordWrap(true);
    layout->addWidget(m_presetTitle);
    auto *scroll = new QScrollArea;
    scroll->setObjectName(QStringLiteral("presetSlotsScroll"));
    scroll->setWidgetResizable(true);
    scroll->setFrameShape(QFrame::NoFrame);
    m_presetSlots = new QWidget;
    m_presetSlots->setObjectName(QStringLiteral("presetSlots"));
    m_presetSlotsLayout = new QVBoxLayout(m_presetSlots);
    m_presetSlotsLayout->setContentsMargins(0, 0, 4, 0);
    m_presetSlotsLayout->setSpacing(6);
    m_presetSlotsLayout->addStretch(1);
    scroll->setWidget(m_presetSlots);
    layout->addWidget(scroll, 1);
    return page;
}

QWidget *ProjectLauncher::buildClone() {
    auto *page = new QWidget;
    auto *form = new QFormLayout(page);
    form->setContentsMargins(0, 4, 0, 0);
    form->setVerticalSpacing(10);
    form->setLabelAlignment(Qt::AlignRight | Qt::AlignVCenter);
    m_cloneUrl = new QLineEdit;
    m_cloneUrl->setObjectName(QStringLiteral("cloneUrl"));
    m_cloneUrl->setPlaceholderText(QStringLiteral("https://github.com/you/film.git"));
    form->addRow(QStringLiteral("Repository"), m_cloneUrl);
    auto *locationRow = new QHBoxLayout;
    m_cloneLocation = new QLineEdit(defaultLocation());
    m_cloneLocation->setObjectName(QStringLiteral("cloneLocation"));
    locationRow->addWidget(m_cloneLocation, 1);
    auto *pick = chooseButton(QStringLiteral("cloneLocationChoose"));
    connect(pick, &QPushButton::clicked, this, [this] {
        const QString path = QFileDialog::getExistingDirectory(this, QStringLiteral("Clone into"), m_cloneLocation->text());
        if (!path.isEmpty()) m_cloneLocation->setText(path);
    });
    locationRow->addWidget(pick);
    form->addRow(QStringLiteral("Location"), locationRow);
    m_cloneDestination = new QLabel;
    m_cloneDestination->setObjectName(QStringLiteral("launcherHint"));
    m_cloneDestination->setWordWrap(true);
    form->addRow(QString(), m_cloneDestination);
    auto *note = new QLabel(QStringLiteral("Kerf configures the repository so project.kerf merges by clip and track, "
                                           "and downloads Git LFS media when it is installed."));
    note->setObjectName(QStringLiteral("launcherHint"));
    note->setWordWrap(true);
    form->addRow(QString(), note);
    const auto changed = [this] {
        const QString folder = cloneFolder();
        m_cloneDestination->setText(folder.isEmpty() ? QString()
                                                     : QStringLiteral("Clones into %1").arg(homeRelative(folder)));
        updateChoose();
    };
    connect(m_cloneUrl, &QLineEdit::textChanged, this, changed);
    connect(m_cloneLocation, &QLineEdit::textChanged, this, changed);
    return page;
}

QWidget *ProjectLauncher::buildDetails() {
    m_details = new QWidget;
    m_details->setObjectName(QStringLiteral("launcherDetails"));
    auto *layout = new QVBoxLayout(m_details);
    layout->setContentsMargins(0, 8, 0, 0);
    layout->setSpacing(6);
    auto *toggleRow = new QHBoxLayout;
    toggleRow->setSpacing(10);
    m_detailsToggle = new QToolButton;
    m_detailsToggle->setObjectName(QStringLiteral("launcherDetailsToggle"));
    m_detailsToggle->setCheckable(true);
    toggleRow->addWidget(m_detailsToggle);
    m_detailsSummary = new QLabel;
    m_detailsSummary->setObjectName(QStringLiteral("launcherHint"));
    toggleRow->addWidget(m_detailsSummary, 1);
    layout->addLayout(toggleRow);

    m_detailsBody = new QWidget;
    m_detailsBody->setObjectName(QStringLiteral("launcherDetailsBody"));
    m_detailsBody->setAttribute(Qt::WA_StyledBackground);
    auto *form = new QFormLayout(m_detailsBody);
    form->setContentsMargins(14, 12, 14, 12);
    form->setVerticalSpacing(8);
    form->setLabelAlignment(Qt::AlignRight | Qt::AlignVCenter);
    const QString location = defaultLocation();
    m_name = new QLineEdit(untitledName(location));
    m_name->setObjectName(QStringLiteral("launcherName"));
    form->addRow(QStringLiteral("Name"), m_name);
    auto *locationRow = new QHBoxLayout;
    m_location = new QLineEdit(location);
    m_location->setObjectName(QStringLiteral("launcherLocation"));
    locationRow->addWidget(m_location, 1);
    auto *pick = chooseButton(QStringLiteral("launcherLocationChoose"));
    connect(pick, &QPushButton::clicked, this, [this] {
        const QString path = QFileDialog::getExistingDirectory(this, QStringLiteral("Save projects in"), m_location->text());
        if (!path.isEmpty()) m_location->setText(path);
    });
    locationRow->addWidget(pick);
    form->addRow(QStringLiteral("Location"), locationRow);
    m_rate = new QComboBox;
    m_rate->setObjectName(QStringLiteral("launcherRate"));
    for (const char *rate : {"23.976", "24", "25", "29.97", "30", "60"})
        m_rate->addItem(QStringLiteral("%1 fps").arg(QString::fromLatin1(rate)), QString::fromLatin1(rate));
    m_rate->setCurrentIndex(std::max(0, m_rate->findData(QSettings().value(QStringLiteral("projects/rate"), QStringLiteral("30")).toString())));
    form->addRow(QStringLiteral("Frame Rate"), m_rate);
    auto *sharing = new QVBoxLayout;
    sharing->setSpacing(5);
    m_git = new QCheckBox(QStringLiteral("Share with Git"));
    m_git->setObjectName(QStringLiteral("launcherGit"));
    m_git->setToolTip(QStringLiteral("Make the project a Git repository, so collaborators pull, merge, and push its edits."));
    sharing->addWidget(m_git);
    m_github = new QCheckBox(QStringLiteral("Create a private GitHub repository"));
    m_github->setObjectName(QStringLiteral("launcherGitHub"));
    m_github->setEnabled(false);
    sharing->addWidget(m_github);
    m_remote = new QLineEdit;
    m_remote->setObjectName(QStringLiteral("launcherRemote"));
    m_remote->setPlaceholderText(QStringLiteral("or push to a remote: git@github.com:you/film.git"));
    sharing->addWidget(m_remote);
    m_gitHint = new QLabel(QStringLiteral("Looking for Git and the GitHub CLI…"));
    m_gitHint->setObjectName(QStringLiteral("launcherHint"));
    m_gitHint->setWordWrap(true);
    sharing->addWidget(m_gitHint);
    form->addRow(QStringLiteral("Sharing"), sharing);
    m_destination = new QLabel;
    m_destination->setObjectName(QStringLiteral("launcherHint"));
    form->addRow(QString(), m_destination);
    layout->addWidget(m_detailsBody);

    const bool open = QSettings().value(QStringLiteral("launcher/details"), true).toBool();
    m_detailsToggle->setChecked(open);
    m_detailsBody->setVisible(open);
    m_detailsToggle->setText(open ? QStringLiteral("▾ Details") : QStringLiteral("▸ Details"));
    connect(m_detailsToggle, &QToolButton::toggled, this, [this](bool show) {
        m_detailsBody->setVisible(show);
        m_detailsToggle->setText(show ? QStringLiteral("▾ Details") : QStringLiteral("▸ Details"));
        QSettings().setValue(QStringLiteral("launcher/details"), show);
    });
    const auto changed = [this] {
        updateDetailsSummary();
        updateChoose();
    };
    connect(m_name, &QLineEdit::textChanged, this, changed);
    connect(m_location, &QLineEdit::textChanged, this, changed);
    connect(m_rate, &QComboBox::currentIndexChanged, this, changed);
    connect(m_git, &QCheckBox::toggled, this, [this, changed](bool on) {
        if (!on) {
            m_github->setChecked(false);
            m_remote->clear();
        }
        m_remote->setEnabled(!m_github->isChecked());
        changed();
    });
    connect(m_github, &QCheckBox::toggled, this, [this, changed](bool on) {
        if (on) m_git->setChecked(true);
        m_remote->setEnabled(!on);
        changed();
    });
    connect(m_remote, &QLineEdit::textChanged, this, [this, changed](const QString &text) {
        if (!text.trimmed().isEmpty()) m_git->setChecked(true);
        changed();
    });
    updateDetailsSummary();
    return m_details;
}

QWidget *ProjectLauncher::buildFooter() {
    auto *footer = new QFrame;
    footer->setObjectName(QStringLiteral("launcherFooter"));
    auto *layout = new QHBoxLayout(footer);
    layout->setContentsMargins(16, 10, 16, 10);
    layout->setSpacing(10);
    m_openExisting = new QPushButton(QStringLiteral("Open an Existing Project…"));
    m_openExisting->setObjectName(QStringLiteral("launcherOpenExisting"));
    connect(m_openExisting, &QPushButton::clicked, this, &ProjectLauncher::openExisting);
    layout->addWidget(m_openExisting);
    m_progress = new QProgressBar;
    m_progress->setObjectName(QStringLiteral("launcherProgress"));
    m_progress->setRange(0, 0);
    m_progress->setTextVisible(false);
    m_progress->setFixedWidth(90);
    m_progress->hide();
    layout->addWidget(m_progress);
    m_problem = new QLabel;
    m_problem->setObjectName(QStringLiteral("launcherProblem"));
    m_problem->setWordWrap(true);
    layout->addWidget(m_problem, 1);
    m_cancel = new QPushButton(QStringLiteral("Cancel"));
    m_cancel->setObjectName(QStringLiteral("launcherCancel"));
    connect(m_cancel, &QPushButton::clicked, this, &QDialog::reject);
    layout->addWidget(m_cancel);
    m_choose = new QPushButton(QStringLiteral("Choose"));
    m_choose->setObjectName(QStringLiteral("launcherChoose"));
    m_choose->setDefault(true);
    m_choose->setMinimumWidth(96);
    connect(m_choose, &QPushButton::clicked, this, &ProjectLauncher::choose);
    layout->addWidget(m_choose);
    return footer;
}

void ProjectLauncher::showPage(Page page) {
    {
        const QSignalBlocker blocker(m_sidebar);
        m_sidebar->setCurrentRow(page);
    }
    m_pages->setCurrentIndex(page);
    static const char *const headings[][2] = {
        {"Choose a template", "A project is a folder: project.kerf, a media/ folder, and a presets/ library."},
        {"Open a recent project", "Double-click a project to open it."},
        {"Start from a shared preset",
         "Fill the preset's media slots with your own files. Its cuts, effects, and timing stay as they are."},
        {"Clone a shared project", "Copy a project from GitHub or any Git remote, with its history."},
    };
    m_heading->setText(QString::fromUtf8(headings[page][0]));
    m_subheading->setText(QString::fromUtf8(headings[page][1]));
    m_details->setVisible(page == NewProject || page == FromPreset);
    QSettings().setValue(QStringLiteral("launcher/page"), int(page));
    updateChoose();
}

QStringList ProjectLauncher::recentProjects() {
    QStringList recent;
    for (const QString &path : QSettings().value(kRecentKey).toStringList())
        if (!path.isEmpty() && !recent.contains(path)) recent.append(path);
    return recent.mid(0, kRecentLimit);
}

void ProjectLauncher::rememberProject(const QString &path) {
    if (path.isEmpty()) return;
    const QString clean = QDir::cleanPath(QFileInfo(path).absoluteFilePath());
    QStringList recent = recentProjects();
    recent.removeAll(clean);
    recent.prepend(clean);
    QSettings().setValue(kRecentKey, recent.mid(0, kRecentLimit));
}

void ProjectLauncher::clearRecentProjects() {
    QSettings().remove(kRecentKey);
}

void ProjectLauncher::reloadRecent() {
    m_recent->clear();
    for (const QString &path : recentProjects()) {
        const QJsonObject info = m_inspectProject(QJsonObject{{QStringLiteral("path"), path}});
        const bool exists = info.contains(QStringLiteral("exists")) ? info.value(QStringLiteral("exists")).toBool()
                                                                     : QFileInfo(path).isDir();
        QString name = info.value(QStringLiteral("name")).toString();
        if (name.isEmpty()) name = QFileInfo(path).fileName();
        QStringList details{homeRelative(path)};
        if (!exists) {
            details << QStringLiteral("Not found");
        } else {
            const QString branch = info.value(QStringLiteral("branch")).toString();
            if (!branch.isEmpty()) details << QStringLiteral("⎇ %1").arg(branch);
            const QDateTime modified = QDateTime::fromString(info.value(QStringLiteral("modified")).toString(), Qt::ISODate);
            if (modified.isValid()) details << QStringLiteral("Edited %1").arg(modified.toLocalTime().toString(QStringLiteral("d MMM yyyy, HH:mm")));
            const int presets = info.value(QStringLiteral("presets")).toInt();
            if (presets > 0) details << (presets == 1 ? QStringLiteral("1 preset") : QStringLiteral("%1 presets").arg(presets));
        }
        auto *item = new QListWidgetItem(projectIcon(exists), QStringLiteral("%1\n%2").arg(name, details.join(QStringLiteral("  ·  "))));
        item->setData(kPathRole, path);
        item->setData(kExistsRole, exists);
        item->setToolTip(path);
        if (!exists) item->setForeground(QColor("#6f6f6f"));
        m_recent->addItem(item);
    }
    if (m_recent->count() == 0) {
        auto *empty = new QListWidgetItem(QStringLiteral("No recent projects yet. Projects you open appear here."));
        empty->setFlags(Qt::NoItemFlags);
        m_recent->addItem(empty);
    } else {
        m_recent->setCurrentRow(0);
    }
}

void ProjectLauncher::reloadPresetChoices() {
    m_presetChoices->clear();
    for (const QString &project : recentProjects()) {
        const QDir presets(QDir(project).filePath(QStringLiteral("presets")));
        for (const QFileInfo &file : presets.entryInfoList({QStringLiteral("*.kerfpreset")}, QDir::Files, QDir::Name)) {
            auto *item = new QListWidgetItem(QStringLiteral("%1\nfrom %2").arg(file.completeBaseName(), QFileInfo(project).fileName()));
            item->setData(kPathRole, file.absoluteFilePath());
            item->setToolTip(file.absoluteFilePath());
            m_presetChoices->addItem(item);
        }
    }
    if (m_presetChoices->count() == 0) {
        auto *empty = new QListWidgetItem(QStringLiteral("Presets saved in your projects' presets/ folders appear here."));
        empty->setFlags(Qt::NoItemFlags);
        m_presetChoices->addItem(empty);
    }
}

void ProjectLauncher::loadPreset(const QString &path) {
    showPreset(path, m_inspectPreset(QJsonObject{{QStringLiteral("path"), path}}));
}

void ProjectLauncher::showPreset(const QString &path, const QJsonObject &preset) {
    while (m_presetSlotsLayout->count() > 1) {
        QLayoutItem *item = m_presetSlotsLayout->takeAt(0);
        if (QWidget *widget = item->widget()) widget->deleteLater();
        delete item;
    }
    m_slotOrder.clear();
    m_slotFiles.clear();
    m_slotFits.clear();
    m_slotStatus.clear();
    m_slotStarts.clear();
    m_presetTexts.clear();
    m_presetPath = path;
    const QString error = preset.value(QStringLiteral("error")).toString();
    m_preset = error.isEmpty() ? preset : QJsonObject{};
    if (!error.isEmpty()) {
        m_presetTitle->setText(QStringLiteral("%1 could not be read: %2").arg(QFileInfo(path).fileName(), error));
        updateChoose();
        return;
    }
    const QJsonArray mediaSlots = preset.value(QStringLiteral("slots")).toArray();
    m_presetTitle->setText(QStringLiteral("%1  ·  %2 MEDIA SLOT%3")
                               .arg(preset.value(QStringLiteral("name")).toString(QFileInfo(path).completeBaseName()).toUpper())
                               .arg(mediaSlots.size())
                               .arg(mediaSlots.size() == 1 ? QString() : QStringLiteral("S")));
    int row = 0;
    for (const QJsonValue &value : mediaSlots) {
        const QJsonObject slot = value.toObject();
        const QString id = slot.value(QStringLiteral("id")).toString();
        m_slotOrder.append(id);
        auto *frame = new QFrame;
        frame->setObjectName(QStringLiteral("presetSlot"));
        auto *grid = new QGridLayout(frame);
        grid->setContentsMargins(12, 8, 10, 8);
        grid->setHorizontalSpacing(10);
        grid->setVerticalSpacing(2);
        auto *name = new QLabel(id);
        name->setObjectName(QStringLiteral("presetSlotName"));
        grid->addWidget(name, 0, 0);
        const bool video = slot.value(QStringLiteral("video")).toBool();
        const bool audio = slot.value(QStringLiteral("audio")).toBool();
        QStringList needs{QStringLiteral("%1 s").arg(slot.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 2)};
        if (video || audio)
            needs << (video && audio ? QStringLiteral("picture and sound") : video ? QStringLiteral("picture") : QStringLiteral("sound"));
        const int clips = slot.value(QStringLiteral("clips")).toInt();
        if (clips > 0) needs << (clips == 1 ? QStringLiteral("1 clip") : QStringLiteral("%1 clips").arg(clips));
        auto *detail = new QLabel(QStringLiteral("Needs ") + needs.join(QStringLiteral(" · ")));
        detail->setObjectName(QStringLiteral("launcherHint"));
        grid->addWidget(detail, 1, 0);
        auto *status = new QLabel(QStringLiteral("No file chosen"));
        status->setObjectName(QStringLiteral("presetSlotStatus_%1").arg(id));
        status->setProperty("fit", QStringLiteral("none"));
        grid->addWidget(status, 0, 1, 2, 1);
        auto *start = new QDoubleSpinBox;
        start->setObjectName(QStringLiteral("presetSlotFrom_%1").arg(id));
        start->setPrefix(QStringLiteral("from "));
        start->setSuffix(QStringLiteral(" s"));
        start->setDecimals(2);
        start->setRange(0, 0);
        start->setEnabled(false);
        start->setToolTip(QStringLiteral("Where the slot starts in the chosen file"));
        grid->addWidget(start, 0, 2, 2, 1);
        auto *pick = new QPushButton(QStringLiteral("Choose File…"));
        pick->setObjectName(QStringLiteral("presetSlotFile_%1").arg(id));
        connect(pick, &QPushButton::clicked, this, [this, id] { chooseSlotFile(id); });
        grid->addWidget(pick, 0, 3, 2, 1);
        grid->setColumnStretch(0, 1);
        grid->setColumnStretch(1, 1);
        m_slotStatus.insert(id, status);
        m_slotStarts.insert(id, start);
        m_presetSlotsLayout->insertWidget(row++, frame);
    }
    const QJsonObject texts = preset.value(QStringLiteral("texts")).toObject();
    if (!texts.isEmpty()) {
        auto *frame = new QFrame;
        frame->setObjectName(QStringLiteral("presetSlot"));
        auto *form = new QFormLayout(frame);
        form->setContentsMargins(12, 8, 10, 8);
        for (auto it = texts.begin(); it != texts.end(); ++it) {
            auto *text = new QLineEdit(it.value().toString());
            text->setObjectName(QStringLiteral("presetText_%1").arg(it.key()));
            form->addRow(QStringLiteral("Text %1").arg(it.key()), text);
            m_presetTexts.insert(it.key(), text);
        }
        m_presetSlotsLayout->insertWidget(row, frame);
    }
    updateChoose();
}

void ProjectLauncher::chooseSlotFile(const QString &slot) {
    const QString start = m_slotFiles.isEmpty() ? QDir::homePath() : QFileInfo(m_slotFiles.cbegin().value()).absolutePath();
    const QString file = QFileDialog::getOpenFileName(
        this, QStringLiteral("Media for %1").arg(slot), start,
        QStringLiteral("Media files (*.mov *.mp4 *.m4v *.mkv *.webm *.wav *.mp3 *.aif *.aiff);;All files (*)"));
    if (!file.isEmpty()) setSlotFile(slot, file);
}

void ProjectLauncher::setSlotFile(const QString &slot, const QString &file) {
    m_slotFiles.insert(slot, file);
    refit();
}

void ProjectLauncher::refit() {
    QStringList files;
    for (const QString &slot : std::as_const(m_slotOrder))
        if (m_slotFiles.contains(slot) && !files.contains(m_slotFiles.value(slot))) files << m_slotFiles.value(slot);
    if (files.isEmpty()) return;
    QGuiApplication::setOverrideCursor(Qt::WaitCursor);
    const QJsonObject answer = m_inspectPreset(QJsonObject{{QStringLiteral("path"), m_presetPath},
                                                           {QStringLiteral("files"), QJsonArray::fromStringList(files)}});
    QGuiApplication::restoreOverrideCursor();
    const QJsonArray probed = answer.value(QStringLiteral("files")).toArray();
    QHash<QString, QJsonObject> needs;
    for (const QJsonValue &value : m_preset.value(QStringLiteral("slots")).toArray())
        needs.insert(value.toObject().value(QStringLiteral("id")).toString(), value.toObject());
    for (const QString &slot : std::as_const(m_slotOrder)) {
        QLabel *status = m_slotStatus.value(slot);
        if (!status || !m_slotFiles.contains(slot)) continue;
        const QString file = m_slotFiles.value(slot);
        const QString name = QFileInfo(file).fileName();
        QJsonObject entry;
        for (const QJsonValue &value : probed)
            if (QDir::cleanPath(value.toObject().value(QStringLiteral("path")).toString()) == QDir::cleanPath(file))
                entry = value.toObject();
        if (entry.isEmpty() && files.indexOf(file) < probed.size()) entry = probed.at(files.indexOf(file)).toObject();
        bool fits = false;
        QString text;
        if (answer.contains(QStringLiteral("error"))) {
            text = QStringLiteral("✕ %1 could not be checked: %2").arg(name, answer.value(QStringLiteral("error")).toString());
        } else if (entry.isEmpty()) {
            text = QStringLiteral("✕ Kerf could not read %1").arg(name);
        } else if (entry.contains(QStringLiteral("error"))) {
            text = QStringLiteral("✕ %1: %2").arg(name, entry.value(QStringLiteral("error")).toString());
        } else if (entry.value(QStringLiteral("fits")).toArray().contains(slot)) {
            fits = true;
            text = QStringLiteral("✓ %1 · %2 s").arg(name).arg(entry.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 2);
        } else {
            const QJsonObject need = needs.value(slot);
            const QString why = need.value(QStringLiteral("video")).toBool() && !entry.value(QStringLiteral("video")).toBool()
                                    ? QStringLiteral("has no picture")
                                : need.value(QStringLiteral("audio")).toBool() && !entry.value(QStringLiteral("audio")).toBool()
                                    ? QStringLiteral("has no sound")
                                    : QStringLiteral("is shorter than %1 s").arg(need.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 2);
            text = QStringLiteral("✕ %1 %2").arg(name, why);
        }
        m_slotFits.insert(slot, fits);
        // The file's length past what the slot plays; kerfd checks the
        // transition handles when it applies the preset.
        QDoubleSpinBox *start = m_slotStarts.value(slot);
        const double spare = entry.value(QStringLiteral("seconds")).toDouble() - needs.value(slot).value(QStringLiteral("seconds")).toDouble();
        start->setMaximum(fits ? std::max(0.0, spare) : 0.0);
        start->setEnabled(fits && spare > 0);
        status->setText(text);
        status->setToolTip(file);
        status->setProperty("fit", fits ? QStringLiteral("yes") : QStringLiteral("no"));
        repolish(status);
    }
    updateChoose();
}

void ProjectLauncher::setTools(const QJsonObject &tools) {
    m_tools = tools;
    const bool git = tools.value(QStringLiteral("git")).toBool();
    const QString user = tools.value(QStringLiteral("ghUser")).toString();
    const bool signedIn = tools.value(QStringLiteral("gh")).toBool() && !user.isEmpty();
    m_git->setEnabled(git);
    if (!git) m_git->setChecked(false);
    m_github->setEnabled(signedIn);
    if (!signedIn) m_github->setChecked(false);
    m_remote->setEnabled(git && !m_github->isChecked());
    QStringList hints;
    if (!git) {
        hints << QStringLiteral("Install Git to share projects (xcode-select --install).");
    } else {
        hints << (signedIn ? QStringLiteral("Signed in to GitHub as @%1.").arg(user)
                  : tools.value(QStringLiteral("gh")).toBool()
                      ? QStringLiteral("Sign in with `gh auth login` to create a GitHub repository here.")
                      : QStringLiteral("Install the GitHub CLI (brew install gh) to create a GitHub repository here."));
        if (!tools.value(QStringLiteral("gitLfs")).toBool())
            hints << QStringLiteral("Git LFS is not installed: media of 100 MB or more cannot be shared.");
    }
    m_gitHint->setText(hints.join(QLatin1Char(' ')));
    updateChoose();
}

void ProjectLauncher::openExisting() {
    const QString path = QFileDialog::getExistingDirectory(this, QStringLiteral("Open an Existing Project"),
                                                           QFileInfo(defaultLocation()).absolutePath());
    if (path.isEmpty()) return;
    if (!QFileInfo::exists(QDir(path).filePath(QStringLiteral("project.kerf"))) &&
        QMessageBox::question(this, QStringLiteral("Open Project"),
                              QStringLiteral("“%1” has no project.kerf. Start a new project in it?").arg(QFileInfo(path).fileName())) !=
            QMessageBox::Yes)
        return;
    m_chosen = path;
    accept();
}

QString ProjectLauncher::templateId() const {
    return m_templates->currentItem() ? m_templates->currentItem()->data(kPathRole).toString() : QStringLiteral("standard");
}

QString ProjectLauncher::projectFolder() const {
    const QString name = folderName(m_name->text());
    const QString location = expanded(m_location->text());
    if (name.isEmpty() || location.isEmpty()) return {};
    return QDir::cleanPath(QDir(location).filePath(name));
}

QString ProjectLauncher::cloneFolder() const {
    QString url = m_cloneUrl->text().trimmed();
    const QString location = expanded(m_cloneLocation->text());
    while (url.endsWith(QLatin1Char('/'))) url.chop(1);
    QString name = url.mid(std::max(url.lastIndexOf(QLatin1Char('/')), url.lastIndexOf(QLatin1Char(':'))) + 1);
    if (name.endsWith(QStringLiteral(".git"))) name.chop(4);
    if (name.isEmpty() || location.isEmpty()) return {};
    return QDir::cleanPath(QDir(location).filePath(name));
}

QString ProjectLauncher::detailsProblem() const {
    if (folderName(m_name->text()).isEmpty()) return QStringLiteral("Name the project");
    const QString location = expanded(m_location->text());
    if (location.isEmpty()) return QStringLiteral("Choose where to save the project");
    if (QDir::isRelativePath(location)) return QStringLiteral("Choose a full location, such as ~/Movies/Kerf");
    if (occupied(projectFolder()))
        return QStringLiteral("“%1” is already in that location; choose another name").arg(folderName(m_name->text()));
    const QString remote = m_remote->text().trimmed();
    if (!remote.isEmpty() && !isGitAddress(remote)) return QStringLiteral("The remote must be a Git address");
    return {};
}

QString ProjectLauncher::problem() const {
    switch (m_pages->currentIndex()) {
    case NewProject:
        if (!m_templates->currentItem()) return QStringLiteral("Choose a template");
        return detailsProblem();
    case Recent: {
        const QListWidgetItem *item = m_recent->currentItem();
        if (!item || !item->flags().testFlag(Qt::ItemIsEnabled)) return QStringLiteral("Choose a recent project");
        if (!item->data(kExistsRole).toBool()) return QStringLiteral("That project folder is no longer there");
        return {};
    }
    case FromPreset:
        if (m_preset.isEmpty()) return QStringLiteral("Choose a preset");
        for (const QString &slot : m_slotOrder)
            if (!m_slotFiles.contains(slot)) return QStringLiteral("Choose a file for each media slot");
        for (const QString &slot : m_slotOrder)
            if (!m_slotFits.value(slot))
                return QStringLiteral("%1 cannot fill %2").arg(QFileInfo(m_slotFiles.value(slot)).fileName(), slot);
        return detailsProblem();
    case Clone: {
        const QString url = m_cloneUrl->text().trimmed();
        if (url.isEmpty()) return QStringLiteral("Enter the repository's address");
        if (!isGitAddress(url)) return QStringLiteral("Enter a Git address such as https://github.com/you/film.git");
        const QString location = expanded(m_cloneLocation->text());
        if (location.isEmpty() || QDir::isRelativePath(location)) return QStringLiteral("Choose where to put the copy");
        if (occupied(cloneFolder())) return QStringLiteral("“%1” is already in that location").arg(QFileInfo(cloneFolder()).fileName());
        return {};
    }
    }
    return {};
}

void ProjectLauncher::updateChoose() {
    if (!m_choose || !m_pages) return;
    const QString why = m_busy ? QString() : problem();
    m_choose->setEnabled(!m_busy && why.isEmpty());
    m_choose->setToolTip(why);
    if (m_busy) return;
    m_problem->setText(why);
    m_problem->setProperty("state", QStringLiteral("hint"));
    repolish(m_problem);
}

void ProjectLauncher::updateDetailsSummary() {
    if (!m_detailsSummary || !m_rate) return;
    QStringList summary{m_name->text().trimmed().isEmpty() ? QStringLiteral("Unnamed") : m_name->text().trimmed(),
                        homeRelative(expanded(m_location->text())), m_rate->currentText()};
    if (m_github->isChecked()) summary << QStringLiteral("private GitHub repository");
    else if (!m_remote->text().trimmed().isEmpty()) summary << QStringLiteral("Git, pushed to the remote");
    else if (m_git->isChecked()) summary << QStringLiteral("Git");
    m_detailsSummary->setText(summary.join(QStringLiteral("  ·  ")));
    const QString folder = projectFolder();
    m_destination->setText(folder.isEmpty() ? QString() : QStringLiteral("Creates %1").arg(homeRelative(folder)));
}

QJsonObject ProjectLauncher::createRequest() const {
    const QString remote = m_remote->text().trimmed();
    const bool github = m_github->isChecked();
    QJsonObject request{{QStringLiteral("type"), QStringLiteral("create")},
                        {QStringLiteral("name"), m_name->text().trimmed()},
                        {QStringLiteral("location"), expanded(m_location->text())},
                        {QStringLiteral("template"), templateId()},
                        {QStringLiteral("rate"), m_rate->currentData().toString()},
                        {QStringLiteral("git"), m_git->isChecked() || github || !remote.isEmpty()}};
    if (github) request.insert(QStringLiteral("github"), QJsonObject{{QStringLiteral("private"), true}});
    else if (!remote.isEmpty()) request.insert(QStringLiteral("remote"), remote);
    // A bare name is looked up the way the bridge would look it up anyway.
    if (m_kerfd.contains(QLatin1Char('/'))) request.insert(QStringLiteral("kerfd"), QFileInfo(m_kerfd).absoluteFilePath());
    return request;
}

void ProjectLauncher::choose() {
    if (m_busy || !problem().isEmpty()) {
        updateChoose();
        return;
    }
    const int page = m_pages->currentIndex();
    if (page == Recent) {
        m_chosen = m_recent->currentItem()->data(kPathRole).toString();
        accept();
        return;
    }
    QJsonObject request;
    QString message;
    if (page == Clone) {
        request = QJsonObject{{QStringLiteral("type"), QStringLiteral("clone")},
                              {QStringLiteral("url"), m_cloneUrl->text().trimmed()},
                              {QStringLiteral("location"), expanded(m_cloneLocation->text())}};
        message = QStringLiteral("Cloning %1…").arg(QFileInfo(cloneFolder()).fileName());
    } else {
        request = createRequest();
        message = QStringLiteral("Making %1…").arg(folderName(m_name->text()));
        QSettings settings;
        settings.setValue(QStringLiteral("projects/template"), templateId());
        settings.setValue(QStringLiteral("projects/rate"), m_rate->currentData().toString());
        if (page == FromPreset) {
            QJsonObject bindings;
            for (const QString &slot : std::as_const(m_slotOrder)) {
                QJsonObject binding{{QStringLiteral("file"), m_slotFiles.value(slot)}};
                const double from = m_slotStarts.value(slot)->value();
                if (from > 0) binding.insert(QStringLiteral("from"), QStringLiteral("%1s").arg(from, 0, 'f', 2));
                bindings.insert(slot, binding);
            }
            QJsonObject preset{{QStringLiteral("path"), m_presetPath}, {QStringLiteral("bindings"), bindings}};
            QJsonObject texts;
            for (auto it = m_presetTexts.cbegin(); it != m_presetTexts.cend(); ++it)
                if (it.value()->isModified()) texts.insert(it.key(), it.value()->text());
            if (!texts.isEmpty()) preset.insert(QStringLiteral("texts"), texts);
            request.insert(QStringLiteral("preset"), preset);
            message = QStringLiteral("Making %1 from the preset…").arg(folderName(m_name->text()));
        }
    }
    setBusy(true, message);
    runTask(request, [this](const QJsonObject &status) { handleTaskStatus(status); });
}

void ProjectLauncher::runTask(const QJsonObject &request, std::function<void(const QJsonObject &)> update) {
    m_requests.append(request);
#ifdef KERF_QT_TESTING
    Q_UNUSED(update);
#else
    const QByteArray json = QJsonDocument(request).toJson(QJsonDocument::Compact);
    void *task = kerf_task_start(json.constData());
    if (!task) {
        update(QJsonObject{{QStringLiteral("status"), QStringLiteral("failed")},
                           {QStringLiteral("error"), QStringLiteral("Kerf could not start the task.")}});
        return;
    }
    auto *timer = new QTimer(this);
    timer->setInterval(100);
    m_tasks.insert(timer, task);
    connect(timer, &QTimer::timeout, this, [this, timer, update] {
        void *running = m_tasks.value(timer);
        char *raw = kerf_task_poll(running);
        QJsonObject status{{QStringLiteral("status"), QStringLiteral("failed")},
                           {QStringLiteral("error"), QStringLiteral("The task stopped answering.")}};
        if (raw) {
            const QJsonDocument document = QJsonDocument::fromJson(QByteArray(raw));
            kerf_bridge_string_free(raw);
            if (document.isObject()) status = document.object();
        }
        if (status.value(QStringLiteral("status")).toString() != QStringLiteral("running")) {
            timer->stop();
            m_tasks.remove(timer);
            kerf_task_free(running);
            timer->deleteLater();
        }
        update(status);
    });
    timer->start();
#endif
}

void ProjectLauncher::handleTaskStatus(const QJsonObject &status) {
    const QString state = status.value(QStringLiteral("status")).toString();
    if (state == QStringLiteral("running")) {
        const QString message = status.value(QStringLiteral("message")).toString();
        if (!message.isEmpty()) m_problem->setText(message);
        return;
    }
    setBusy(false);
    if (state != QStringLiteral("done")) {
        showFailure(status.value(QStringLiteral("error")).toString(QStringLiteral("Kerf could not make the project.")));
        return;
    }
    const QString path = status.value(QStringLiteral("path")).toString();
    if (path.isEmpty()) {
        showFailure(QStringLiteral("The task finished without a project folder."));
        return;
    }
    const bool cloned = m_pages->currentIndex() == Clone;
    QSettings().setValue(QStringLiteral("projects/location"), expanded((cloned ? m_cloneLocation : m_location)->text()));
    QStringList warnings;
    for (const QJsonValue &warning : status.value(QStringLiteral("warnings")).toArray()) warnings << warning.toString();
#ifndef KERF_QT_TESTING
    if (!warnings.isEmpty())
        QMessageBox::warning(this, QStringLiteral("Project ready"), warnings.join(QStringLiteral("\n\n")));
#endif
    m_chosen = path;
    accept();
}

void ProjectLauncher::setBusy(bool busy, const QString &message) {
    m_busy = busy;
    m_progress->setVisible(busy);
    m_sidebar->setEnabled(!busy);
    m_pages->setEnabled(!busy);
    m_details->setEnabled(!busy);
    m_openExisting->setEnabled(!busy);
    if (busy) {
        m_problem->setText(message);
        m_problem->setProperty("state", QStringLiteral("busy"));
        repolish(m_problem);
    }
    updateChoose();
}

void ProjectLauncher::showFailure(const QString &message) {
    m_problem->setText(message);
    m_problem->setProperty("state", QStringLiteral("error"));
    repolish(m_problem);
}

} // namespace kerf
