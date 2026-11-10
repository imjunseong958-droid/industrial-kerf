// MainWindow's media bin, presets and the browser (mockup-spec 6 and 8).
// Q-BINBROWSER (W2); the region's own state and engine traffic are in
// ui/wiring/bin_wiring.*.

#include "main_window.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "inspector_panel.h"
#include "effects_panel.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "stage_shell.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#ifdef Q_OS_MACOS
#include "macos_window.h"
#endif

#include <QAbstractTextDocumentLayout>
#include <QActionGroup>
#include <QApplication>
#include <QButtonGroup>
#include <QClipboard>
#include <QColorDialog>
#include <QCursor>
#include <QDateTime>
#include <QDesktopServices>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontDatabase>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QIcon>
#include <QImage>
#include <QJsonDocument>
#include <QKeySequence>
#include <QLayout>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QPolygonF>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QProcess>
#include <QRegularExpression>
#include <QSaveFile>
#include <QScrollBar>
#include <QSettings>
#include <QScreen>
#include <QShortcut>
#include <QShowEvent>
#include <QSignalBlocker>
#include <QTextBlock>
#include <QTextDocument>
#include <QTextFrame>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>
#include <QWindow>


#include <algorithm>
#include <numeric>
#include <limits>
#include <utility>
#include "wiring/main_window_shared.h"
#include "browser_panel.h"
#include "deliver_tray.h"
#include "editor_bus.h"
#include "finish_tray.h"
#include "inspector_column.h"
#include "wiring/bin_wiring.h"

namespace kerf {

using namespace window_parts;

void MainWindow::exportEditingPreset() {
    QFileDialog dialog(this, QStringLiteral("Export editing preset"),
        QDir(m_projectDirectory).filePath(QStringLiteral("editing.kerfpreset")), QStringLiteral("Kerf editing preset (*.kerfpreset)"));
    dialog.setAcceptMode(QFileDialog::AcceptSave);
    dialog.setDefaultSuffix(QStringLiteral("kerfpreset"));
    if (dialog.exec() != QDialog::Accepted) return;
    const QString path = dialog.selectedFiles().value(0);
    if (QFileInfo(path).suffix() != QStringLiteral("kerfpreset")) {
        showError(QStringLiteral("Save editing presets with the .kerfpreset extension."));
        return;
    }
    execute(QStringLiteral("file.exportPreset"), QJsonObject{{QStringLiteral("path"), path}});
}

void MainWindow::importEditingPreset() {
    const QString path = QFileDialog::getOpenFileName(this, QStringLiteral("Import editing preset"),
        m_projectDirectory, QStringLiteral("Kerf editing preset (*.kerfpreset)"));
    if (!path.isEmpty()) sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("preset_inspect")},
                                               {QStringLiteral("path"), path}});
}

void MainWindow::showPresetBindings(const QString &path, const QJsonObject &preset) {
    QDialog dialog(this);
    dialog.setObjectName(QStringLiteral("presetBindingsDialog"));
    dialog.setWindowTitle(QStringLiteral("Apply preset — %1").arg(preset.value(QStringLiteral("name")).toString()));
    dialog.setMinimumWidth(560);
    auto *layout = new QVBoxLayout(&dialog);
    layout->setContentsMargins(20, 20, 20, 20);
    layout->setSpacing(12);
    auto *intro = new QLabel(QStringLiteral("Choose media for each slot. Kerf keeps the preset's timing, effects and animation. "
        "Applying replaces this sequence as a proposal for you to review."));
    intro->setWordWrap(true);
    layout->addWidget(intro);
    auto *scroll = new QScrollArea;
    scroll->setWidgetResizable(true);
    scroll->setFrameShape(QFrame::NoFrame);
    auto *contents = new QWidget;
    auto *form = new QFormLayout(contents);
    form->setContentsMargins(0, 0, 0, 0);
    form->setVerticalSpacing(8);
    form->setHorizontalSpacing(12);
    form->setLabelAlignment(Qt::AlignRight | Qt::AlignTop);
    QHash<QString, QComboBox *> media;
    QHash<QString, QDoubleSpinBox *> starts;
    QHash<QString, QLineEdit *> texts;
    QHash<QString, QJsonObject> sources;
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
        sources.insert(value.toObject().value(QStringLiteral("id")).toString(), value.toObject());
    // Kerf lists the media able to fill each slot and the latest frame each can
    // start from. Starts are shown in seconds but sent as whole frames.
    constexpr int latestFrameRole = Qt::UserRole + 1;
    constexpr int fpsRole = Qt::UserRole + 2;
    const QString root = preset.value(QStringLiteral("sequence")).toString();
    for (const QJsonValue &value : preset.value(QStringLiteral("slots")).toArray()) {
        const QJsonObject slot = value.toObject();
        const QString id = slot.value(QStringLiteral("id")).toString();
        auto *choice = new QComboBox;
        choice->setObjectName(QStringLiteral("presetMedia_%1").arg(id));
        choice->addItem(QStringLiteral("Choose imported media…"), QString());
        for (const QJsonValue &fitValue : slot.value(QStringLiteral("fits")).toArray()) {
            const QJsonObject fit = fitValue.toObject();
            const QJsonObject source = sources.value(fit.value(QStringLiteral("media")).toString());
            if (source.isEmpty()) continue;
            const QString sourceId = source.value(QStringLiteral("id")).toString();
            choice->addItem(QStringLiteral("%1 — %2").arg(sourceId,
                QFileInfo(source.value(QStringLiteral("path")).toString()).fileName()), sourceId);
            choice->setItemData(choice->count() - 1, fit.value(QStringLiteral("latestFrame")).toInteger(), latestFrameRole);
            choice->setItemData(choice->count() - 1, source.value(QStringLiteral("fps")).toDouble(), fpsRole);
        }
        if (choice->count() == 1) choice->setItemText(0, QStringLiteral("Import matching media first"));
        auto *from = new StepperSpinBox;
        from->setObjectName(QStringLiteral("presetFrom_%1").arg(id));
        // One width for every slot, so the steppers line up down the column.
        from->setFixedWidth(106);
        from->setDecimals(3);
        from->setSuffix(QStringLiteral(" s"));
        from->setRange(0, 0);
        from->setToolTip(QStringLiteral("Start in the replacement media, including the preset's transition handles"));
        connect(choice, &QComboBox::currentIndexChanged, from, [choice, from] {
            const double fps = choice->currentData(fpsRole).toDouble();
            from->setSingleStep(fps > 0 ? 1 / fps : 1);
            from->setMaximum(fps > 0 ? choice->currentData(latestFrameRole).toDouble() / fps : 0);
        });
        QStringList uses;
        for (const QJsonValue &clipValue : slot.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            QString use = QStringLiteral("%1 %2").arg(clip.value(QStringLiteral("track")).toString(),
                                                      clip.value(QStringLiteral("at")).toString());
            const QString sequence = clip.value(QStringLiteral("sequence")).toString();
            if (sequence != root) use = QStringLiteral("%1 › %2").arg(sequence, use);
            const QString name = clip.value(QStringLiteral("name")).toString();
            if (!name.isEmpty()) use += QStringLiteral(" “%1”").arg(name);
            uses << use;
        }
        auto *usedBy = new QLabel(uses.join(QStringLiteral(" · ")));
        usedBy->setObjectName(QStringLiteral("muted"));
        usedBy->setWordWrap(true);
        auto *row = new QWidget;
        auto *rowLayout = new QGridLayout(row);
        rowLayout->setContentsMargins(0, 0, 0, 0);
        rowLayout->setHorizontalSpacing(8);
        rowLayout->setVerticalSpacing(4);
        rowLayout->addWidget(choice, 0, 0);
        rowLayout->addWidget(from, 0, 1);
        rowLayout->addWidget(usedBy, 1, 0, 1, 2);
        rowLayout->setColumnStretch(0, 1);
        rowLayout->setColumnStretch(1, 0);
        form->addRow(QStringLiteral("%1 · %2 s").arg(id).arg(slot.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 3), row);
        media.insert(id, choice);
        starts.insert(id, from);
    }
    // Text slots by number, as the launcher shows them; their clip ids are
    // the document's names, in the tooltip.
    const QJsonObject textSlots = preset.value(QStringLiteral("textSlots")).toObject();
    int textNumber = 0;
    for (auto it = textSlots.begin(); it != textSlots.end(); ++it) {
        auto *text = new QLineEdit(it.value().toString());
        text->setObjectName(QStringLiteral("presetText_%1").arg(it.key()));
        text->setToolTip(it.key());
        form->addRow(QStringLiteral("Text %1").arg(++textNumber), text);
        texts.insert(it.key(), text);
    }
    scroll->setWidget(contents);
    layout->addWidget(scroll, 1);
    // The preset's source, for those who want it, behind a disclosure.
    auto *details = new QToolButton;
    details->setObjectName(QStringLiteral("disclosure"));
    details->setText(QStringLiteral("Details"));
    details->setCheckable(true);
    details->setAutoRaise(true);
    details->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    details->setIconSize(QSize(11, 11));
    details->setIcon(disclosureIcon(false, editorTheme().textSecondary));
    layout->addWidget(details, 0, Qt::AlignLeft);
    auto *code = new QPlainTextEdit(preset.value(QStringLiteral("document")).toString());
    code->setObjectName(QStringLiteral("presetCode"));
    code->setReadOnly(true);
    code->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    code->setMaximumHeight(150);
    code->hide();
    layout->addWidget(code);
    // As tall as what it shows, as macOS dialogs are: the slots up to most
    // of the screen (they scroll past that), and the source when Details
    // is open.
    const auto fit = [&dialog, layout] {
        layout->activate();
        dialog.resize(dialog.width(), layout->hasHeightForWidth() ? layout->totalHeightForWidth(dialog.width())
                                                                  : layout->totalSizeHint().height());
    };
    connect(details, &QToolButton::toggled, code, [details, code, fit](bool open) {
        code->setVisible(open);
        details->setIcon(disclosureIcon(open, editorTheme().textSecondary));
        fit();
    });
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Create Proposal"));
    buttons->button(QDialogButtonBox::Ok)->setDefault(true);
    const auto updateReady = [buttons, media] {
        bool ready = true;
        for (auto *choice : media) ready = ready && !choice->currentData().toString().isEmpty();
        buttons->button(QDialogButtonBox::Ok)->setEnabled(ready);
    };
    for (auto *choice : media) connect(choice, &QComboBox::currentIndexChanged, buttons, updateReady);
    updateReady();
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addWidget(buttons);
    dialog.ensurePolished();
    dialog.resize(600, dialog.height());
    const QMargins margins = layout->contentsMargins();
    const int inner = dialog.width() - margins.left() - margins.right();
    const int formHeight = form->hasHeightForWidth() ? form->totalHeightForWidth(inner) : form->totalSizeHint().height();
    const QScreen *screen = dialog.screen();
    scroll->setFixedHeight(std::min(formHeight, screen ? screen->availableGeometry().height() * 6 / 10 : 520));
    fit();
    const QJsonValue revision = revisionValue();
    if (dialog.exec() != QDialog::Accepted) return;
    if (revision != revisionValue()) {
        showError(QStringLiteral("The project changed while choosing preset media. Import the preset again."));
        return;
    }
    QJsonObject bindings;
    for (auto it = media.begin(); it != media.end(); ++it) {
        // Clamp: the shown maximum is rounded to the spin box's decimals.
        const qint64 frame = std::clamp<qint64>(
            qRound64(starts.value(it.key())->value() * it.value()->currentData(fpsRole).toDouble()),
            0, it.value()->currentData(latestFrameRole).toLongLong());
        bindings.insert(it.key(), QJsonObject{{QStringLiteral("media"), it.value()->currentData().toString()},
            {QStringLiteral("from"), QStringLiteral("%1f").arg(frame)}});
    }
    QJsonObject overrides;
    for (auto it = texts.begin(); it != texts.end(); ++it) {
        if (it.value()->isModified()) overrides.insert(it.key(), it.value()->text());
    }
    execute(QStringLiteral("file.applyPreset"), QJsonObject{{QStringLiteral("path"), path},
        {QStringLiteral("bindings"), bindings}, {QStringLiteral("texts"), overrides}});
}

void MainWindow::savePreset() {
    bool ok = false;
    const QString name = QInputDialog::getText(this, QStringLiteral("Save Current as Preset"),
                                               QStringLiteral("Saves the program sequence in presets/, shared with the project.\nName"),
                                               QLineEdit::Normal, QString(), &ok)
                             .trimmed();
    if (!ok || name.isEmpty()) return;
    // The bridge names the file, cleaning the name up, and refuses one that
    // exists; replacing it is asked then.
    m_savingPreset = name;
    execute(QStringLiteral("preset.save"), QJsonObject{{QStringLiteral("name"), name}, {QStringLiteral("replace"), false}});
}

void MainWindow::replaceSavedPreset(const QString &path) {
    const QString name = std::exchange(m_savingPreset, QString());
    if (QMessageBox::question(this, QStringLiteral("Save Current as Preset"),
                              QStringLiteral("%1 already exists. Replace it?").arg(path)) == QMessageBox::Yes)
        execute(QStringLiteral("preset.save"), QJsonObject{{QStringLiteral("name"), name}, {QStringLiteral("replace"), true}});
}

void MainWindow::applyLibraryPreset(const QString &path) {
    // The preset's slots and the media able to fill them come back as
    // preset_inspect, which opens the bindings dialog.
    sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("preset_inspect")}, {QStringLiteral("path"), path}});
}

QString MainWindow::mediaPath(const QString &mediaId) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
        const QJsonObject source = value.toObject();
        if (source.value(QStringLiteral("id")).toString() == mediaId)
            return QDir(m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString());
    }
    return {};
}

void MainWindow::replaceMedia(const QString &mediaId, const QString &path) {
    QString file = path;
    if (file.isEmpty()) {
        file = QFileDialog::getOpenFileName(this, QStringLiteral("Replace %1").arg(mediaId), QFileInfo(mediaPath(mediaId)).absolutePath(),
                                            QStringLiteral("Media files (*.mov *.mp4 *.m4v *.mkv *.webm *.wav *.mp3);;All files (*)"));
        if (file.isEmpty()) return;
    }
    if (execute(QStringLiteral("media.replace"), QJsonObject{{QStringLiteral("media"), mediaId}, {QStringLiteral("path"), file}}))
        addActivity(QStringLiteral("import"), QStringLiteral("Replacing %1 with %2").arg(mediaId, QFileInfo(file).fileName()));
}

QString MainWindow::mediaRecipe(const QString &mediaId) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray()) {
        const QJsonObject source = value.toObject();
        if (source.value(QStringLiteral("id")).toString() == mediaId) return source.value(QStringLiteral("recipe")).toString();
    }
    return {};
}

void MainWindow::makeMedia(const QString &mediaId) {
    execute(QStringLiteral("media.make"), QJsonObject{{QStringLiteral("media"), mediaId}});
}

QString MainWindow::makeQuestion(const QJsonObject &plan) {
    // A recipe is a command someone wrote, perhaps someone else: show all of it.
    return QStringLiteral("Making %1 runs this command on your computer:\n\n%2\n\nIt comes from the recipe %3, and what it writes replaces %4.\n\nRun it?")
        .arg(plan.value(QStringLiteral("media")).toString(), plan.value(QStringLiteral("command")).toString(),
             plan.value(QStringLiteral("recipe")).toString(), plan.value(QStringLiteral("file")).toString());
}

void MainWindow::runMake(const QString &mediaId) {
    if (execute(QStringLiteral("media.make"), QJsonObject{{QStringLiteral("media"), mediaId}, {QStringLiteral("confirmed"), true}}))
        addActivity(QStringLiteral("import"), QStringLiteral("Making %1 from %2").arg(mediaId, mediaRecipe(mediaId)));
}

void MainWindow::revealMedia(const QString &mediaId) {
    const QString path = mediaPath(mediaId);
    if (path.isEmpty()) return;
#if defined(Q_OS_MACOS)
    QProcess::startDetached(QStringLiteral("open"), {QStringLiteral("-R"), path});
#else
    QDesktopServices::openUrl(QUrl::fromLocalFile(QFileInfo(path).absolutePath()));
#endif
}

QMenu *MainWindow::binMenu(const QString &mediaId) {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("binMenu"));
    menu->setToolTipsVisible(true);
    if (!mediaId.isEmpty()) {
        QAction *open = menu->addAction(QStringLiteral("Open in Source Monitor"));
        open->setObjectName(QStringLiteral("binOpen"));
        connect(open, &QAction::triggered, this, [this, mediaId] { openSource(mediaId); });
        menu->addSeparator();
        QAction *replace = menu->addAction(QStringLiteral("Replace Media…"));
        replace->setObjectName(QStringLiteral("binReplace"));
        replace->setEnabled(m_commands->isEnabled(QStringLiteral("media.replace")));
        replace->setToolTip(replace->isEnabled() ? QStringLiteral("Swap the file behind %1 for everyone; its clips keep their timing").arg(mediaId)
                                                 : m_commands->reason(QStringLiteral("media.replace")));
        connect(replace, &QAction::triggered, this, [this, mediaId] { replaceMedia(mediaId); });
        if (const QString recipe = mediaRecipe(mediaId); !recipe.isEmpty()) {
            QAction *make = menu->addAction(QStringLiteral("Make from Recipe…"));
            make->setObjectName(QStringLiteral("binMake"));
            make->setEnabled(m_commands->isEnabled(QStringLiteral("media.make")));
            make->setToolTip(make->isEnabled() ? QStringLiteral("Make %1's file again by running %2; the command is shown first").arg(mediaId, recipe)
                                               : m_commands->reason(QStringLiteral("media.make")));
            connect(make, &QAction::triggered, this, [this, mediaId] { makeMedia(mediaId); });
        }
        QAction *reveal = menu->addAction(QStringLiteral("Reveal in Finder"));
        reveal->setObjectName(QStringLiteral("binReveal"));
        reveal->setEnabled(QFileInfo::exists(mediaPath(mediaId)));
        connect(reveal, &QAction::triggered, this, [this, mediaId] { revealMedia(mediaId); });
        menu->addSeparator();
    }
    menu->addAction(m_commands->action(QStringLiteral("file.import")));
    menu->addAction(m_commands->action(QStringLiteral("collab.consolidate")));
    return menu;
}

// The media bin, the presets and the effects, and the browser that holds them.
void MainWindow::buildBin() {
    m_projectPanel = new ProjectPanel(m_artwork);
    connect(m_projectPanel, &ProjectPanel::openRequested, this, &MainWindow::openSource);
    connect(m_projectPanel, &ProjectPanel::menuRequested, this, [this](const QString &media, const QPoint &at) {
        QMenu *menu = binMenu(media);
        menu->exec(at);
        menu->deleteLater();
    });
    m_presets = new PresetsPanel;
    connect(m_presets, &PresetsPanel::applyRequested, this, &MainWindow::applyLibraryPreset);

    // The effects offered: cards on the selected clip (or the program's
    // clip under the playhead), from the whole catalog, picture and sound;
    // the Inspector edits the picture catalog's parameters.
    m_effects = new EffectsPanel;
    if (char *catalog = kerf_fx_catalog_all()) {
        m_effects->setCatalog(QJsonDocument::fromJson(QByteArray(catalog)).array());
        kerf_bridge_string_free(catalog);
    }
    connect(m_effects, &EffectsPanel::addRequested, this, [this](const QString &fx) {
        const QStringList clips = m_inspector->selection();
        if (clips.isEmpty()) return;
        QJsonArray ops;
        for (const QString &clip : clips)
            ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-add")}, {QStringLiteral("clip"), clip}, {QStringLiteral("fx"), fx}});
        sendEdit(ops);
        // Adding selects nothing new: back to the Inspector, the new effect
        // open and in view.
        if (m_effectsFromInspector && clips.size() == 1) {
            m_effectsFromInspector = false;
            m_inspector->revealEffect(int(selectedClipObject().value(QStringLiteral("effects")).toArray().size()));
            m_shell->setSheet(StageShell::Sheet::Inspector);
            m_inspectorColumn->setPage(InspectorColumn::Page::Inspector);
        }
    });
    connect(m_effects, &EffectsPanel::previewsWanted, this, [this](const QString &clip, const QStringList &effects) {
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("effect_previews")},
                               {QStringLiteral("clip"), clip},
                               {QStringLiteral("at"), m_playhead},
                               {QStringLiteral("effects"), QJsonArray::fromStringList(effects)},
                               {QStringLiteral("width"), EffectsPanel::kPreviewWidth},
                               {QStringLiteral("key"), QString::fromLatin1(EffectsPanel::kCardKey)}});
    });

    // The browser at the band's right: Effects | Titles | Presets.
    m_browserPanel = new BrowserPanel(nullptr, m_bus);
    m_browserPanel->setEffects(m_effects);
    m_browserPanel->setPresets(m_presets);
    // Leaving Effects without adding: a later add made there stays there.
    connect(m_browserPanel, &BrowserPanel::pageChanged, this, [this](BrowserPanel::Page page) {
        if (page != BrowserPanel::Page::Effects) m_effectsFromInspector = false;
        syncPanelCommands();
    });
    // The Titles page, the bin's Index and Roles, and the region's engine traffic.
    m_binWiring->build();
}

} // namespace kerf
