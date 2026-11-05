#include "home_window.h"

#include "components.h"
#include "composer.h"
#include "design_tokens.h"
#include "project_launcher.h"
#include "project_sidebar.h"
#include "stage_motion.h"
#ifdef Q_OS_MACOS
#include "macos_window.h"
#endif

#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QLabel>
#include <QMenu>
#include <QPaintEvent>
#include <QPainter>
#include <QPalette>
#include <QShowEvent>
#include <QTextEdit>
#include <QTimer>
#include <QVBoxLayout>
#include <QWindow>

// On macOS the window runs up under a transparent title bar, as the editor's
// does. Elsewhere the native title bar stays.
#if defined(Q_OS_MACOS) && QT_VERSION >= QT_VERSION_CHECK(6, 9, 0)
#define KERF_HOME_OWN_TITLE_BAR 1
#else
#define KERF_HOME_OWN_TITLE_BAR 0
#endif

namespace kerf {

using namespace tokens;

namespace {

constexpr int kColumn = 720;

// One thing to try: an outline and quiet words, under the question.
class IdeaButton final : public ui::KitButton {
public:
    explicit IdeaButton(const QString &text) {
        setText(text);
        setObjectName(QStringLiteral("homeIdea"));
        setFocusPolicy(Qt::TabFocus);
        setFixedSize(ui::snapWidth(control, text) + 2 * space::s16, height::h32);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QRectF r = rect();
        ui::paintInsetLine(p, r, r.height() / 2.0, tokens::color(isHovered() ? lineh : line2));
        p.setFont(tokens::font(control));
        p.setPen(tokens::color(isHovered() ? ink : ink2));
        p.drawText(r, Qt::AlignCenter, text());
        if (focusVisible()) ui::paintFocusRing(p, r, r.height() / 2.0);
    }
};

} // namespace

QStringList HomeWindow::suggestions() {
    return {QStringLiteral("Make a 60-second vertical cut"), QStringLiteral("Keep only the spoken parts"),
            QStringLiteral("Pull the best moments")};
}

HomeWindow::HomeWindow(const QString &kerfdBinary, QWidget *parent) : QDialog(parent), m_kerfd(kerfdBinary) {
    setObjectName(QStringLiteral("homeWindow"));
    setWindowTitle(QStringLiteral("Industrial Kerf"));
    resize(1500, 940);
    setMinimumSize(860, 560);
#if KERF_HOME_OWN_TITLE_BAR
    setWindowFlag(Qt::ExpandedClientAreaHint);
    setWindowFlag(Qt::NoTitleBarBackgroundHint);
    setAttribute(Qt::WA_ContentsMarginsRespectsSafeArea, false);
#endif
    m_pickFolder = [this] {
        return QFileDialog::getExistingDirectory(this, QStringLiteral("Open Folder"), m_folder.isEmpty() ? QDir::homePath() : QFileInfo(m_folder).absolutePath());
    };

    auto *root = new QHBoxLayout(this);
    root->setContentsMargins(0, 0, 0, 0);
    root->setSpacing(0);

    m_sidebar = new ProjectSidebar;
    m_sidebar->setFixedWidth(layout::sidebar);
    m_sidebar->setConversationsShown(false);
    // The rows sit where the editor's will, so they do not move at the hand-over.
    m_sidebar->setTopInset(KERF_HOME_OWN_TITLE_BAR ? layout::toolbar + space::s8 : space::s8);
    connect(m_sidebar, &ProjectSidebar::projectChosen, this, [this](const QString &path) {
        setFolder(path);
        m_composer->input()->setFocus();
    });
    connect(m_sidebar, &ProjectSidebar::newProjectRequested, this, &HomeWindow::newProject);
    connect(m_sidebar, &ProjectSidebar::openFolderRequested, this, &HomeWindow::chooseFolder);
    root->addWidget(m_sidebar);

    auto *middle = new QVBoxLayout;
    middle->setContentsMargins(space::s16, layout::toolbar, space::s16, layout::toolbar);
    middle->setSpacing(0);
    middle->addStretch(4);

    auto *heading = new QLabel(QStringLiteral("What should we make?"));
    heading->setObjectName(QStringLiteral("homeHeading"));
    m_heading = heading;
    heading->setFont(tokens::font(display));
    heading->setAlignment(Qt::AlignCenter);
    {
        QPalette palette = heading->palette();
        palette.setColor(QPalette::WindowText, tokens::color(ink));
        heading->setPalette(palette);
    }
    middle->addWidget(heading);
    middle->addSpacing(space::s16 + space::s8);

    m_composer = new Composer;
    m_composer->setObjectName(QStringLiteral("homeComposer"));
    m_composer->setDivider(false);
    m_composer->setRoomy();
    m_composer->setMaximumWidth(kColumn);
    m_composer->setOverlayParent(this);
    m_composer->input()->setPlaceholderText(QStringLiteral("Say what to make from this folder"));
    // A request here has no playhead to stamp, nobody to mention, nothing to draw on.
    m_composer->stamp()->hide();
    m_composer->mentionButton()->hide();
    m_composer->drawButton()->hide();
    m_composer->versionsButton()->hide();
    m_folderChip = new ui::GhostButton;
    m_folderChip->setObjectName(QStringLiteral("homeFolder"));
    m_folderChip->setIconName(QStringLiteral("g-folder"));
    m_folderChip->setChevron(true);
    connect(m_folderChip, &QAbstractButton::clicked, this, &HomeWindow::showFolderMenu);
    m_composer->toolRow()->insertWidget(0, m_folderChip);
    connect(m_composer, &Composer::submitRequested, this, &HomeWindow::send);
    connect(m_composer->input(), &QTextEdit::textChanged, this, &HomeWindow::updateControls);
    m_composer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    auto *line = new QHBoxLayout;
    line->setContentsMargins(0, 0, 0, 0);
    line->addStretch(1);
    line->addWidget(m_composer, 100);
    line->addStretch(1);
    middle->addLayout(line);
    middle->addSpacing(space::s8);

    m_ideaRow = new QWidget;
    m_ideaRow->setObjectName(QStringLiteral("homeIdeas"));
    auto *ideas = new QHBoxLayout(m_ideaRow);
    ideas->setContentsMargins(0, 0, 0, 0);
    ideas->setSpacing(space::s8);
    ideas->addStretch(1);
    for (const QString &idea : suggestions()) {
        auto *chip = new IdeaButton(idea);
        connect(chip, &QAbstractButton::clicked, this, [this, idea] { m_composer->fill(idea); });
        ideas->addWidget(chip);
        m_ideas << chip;
    }
    ideas->addStretch(1);
    middle->addWidget(m_ideaRow);
    middle->addSpacing(space::s16);

    m_openOnly = new ui::GhostButton(QStringLiteral("Open without asking"));
    m_openOnly->setObjectName(QStringLiteral("homeOpenOnly"));
    m_openOnly->setMeta(true);
    connect(m_openOnly, &QAbstractButton::clicked, this, [this] { finish(QString()); });
    middle->addWidget(m_openOnly, 0, Qt::AlignHCenter);
    middle->addStretch(5);
    root->addLayout(middle, 1);

    const QStringList recent = ProjectLauncher::recentProjects();
    m_sidebar->setProjects(recent, QString());
    setFolder(m_sidebar->projects().value(0));
}

void HomeWindow::setFolder(const QString &path) {
    m_folder = path.isEmpty() ? QString() : QDir::cleanPath(QFileInfo(path).absoluteFilePath());
    m_sidebar->setProjects(ProjectLauncher::recentProjects(), m_folder);
    m_folderChip->setText(m_folder.isEmpty() ? QStringLiteral("Choose a folder") : QFileInfo(m_folder).fileName());
    m_folderChip->setFixedSize(m_folderChip->sizeHint());
    ui::setTip(m_folderChip, m_folder.isEmpty() ? QStringLiteral("The folder to work in") : m_folder);
    updateControls();
}

void HomeWindow::updateControls() {
    m_composer->sendButton()->setEnabled(!m_composer->text().trimmed().isEmpty());
    m_openOnly->setVisible(!m_folder.isEmpty());
}

void HomeWindow::chooseFolder() {
    const QString path = m_pickFolder();
    if (path.isEmpty()) return;
    setFolder(path);
    m_composer->input()->setFocus();
}

void HomeWindow::newProject() {
    ProjectLauncher launcher(m_kerfd, this);
    if (launcher.exec() != QDialog::Accepted) return;
    setFolder(launcher.chosenPath());
    finish(m_composer->text().trimmed());
}

void HomeWindow::showFolderMenu() {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("homeFolderMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    ui::styleMenu(menu);
    for (const QString &path : m_sidebar->projects()) {
        QAction *action = menu->addAction(QFileInfo(path).fileName());
        action->setCheckable(true);
        action->setChecked(path == m_folder);
        connect(action, &QAction::triggered, this, [this, path] { setFolder(path); });
    }
    if (!menu->isEmpty()) menu->addSeparator();
    connect(menu->addAction(QStringLiteral("Open Folder…")), &QAction::triggered, this, &HomeWindow::chooseFolder);
    ui::popupMenu(menu, m_folderChip);
}

void HomeWindow::send() {
    const QString text = m_composer->text().trimmed();
    if (text.isEmpty()) return;
    if (m_folder.isEmpty()) {
        const QString path = m_pickFolder();
        if (path.isEmpty()) return;
        setFolder(path);
    }
    finish(text);
}

void HomeWindow::finish(const QString &request) {
    if (m_folder.isEmpty()) return;
    m_request = request;
    m_startsProject = !QFileInfo::exists(QDir(m_folder).filePath(QStringLiteral("project.kerf")));
    m_lastLook = grab();
    // It stays on screen until the editor is over it.
    setResult(QDialog::Accepted);
    emit chosen();
}

void HomeWindow::showEvent(QShowEvent *event) {
    QDialog::showEvent(event);
#if KERF_HOME_OWN_TITLE_BAR
    hideNativeWindowTitle(this);
#endif
    m_composer->input()->setFocus();
    // Once it is laid out.
    if (!m_entered) {
        m_entered = true;
        QTimer::singleShot(0, this, &HomeWindow::enter);
    }
}

void HomeWindow::enter() {
    // The room comes together: the question, the box, then what to try.
    moves::rise(m_heading, 0);
    moves::rise(m_composer, 90);
    moves::rise(m_ideaRow, 180);
    moves::rise(m_openOnly, 260);
    moves::fadeIn(m_sidebar, 0, 420);
}

QRect HomeWindow::composerRect() const { return QRect(m_composer->box()->mapTo(this, QPoint()), m_composer->box()->size()); }

void HomeWindow::paintEvent(QPaintEvent *event) {
    QPainter p(this);
    p.fillRect(event->rect(), tokens::color(chrome));
    // One hairline between the folders and the question.
    p.fillRect(QRect(layout::sidebar, 0, layout::seam, height()), tokens::color(line));
}

} // namespace kerf
