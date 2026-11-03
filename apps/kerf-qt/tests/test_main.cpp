// kerf-qt-tests: every test class under tests/, run one after another in a
// single application, as the one EditorTest class used to run.
//
//   kerf-qt-tests -platform offscreen                     every class
//   kerf-qt-tests -platform offscreen widePictureShowsInALaptop themeColorsReadWhereTheyAreDrawn:ink
//                                                         only the classes those slots belong to
//   kerf-qt-tests -functions                              every class's test functions
//   kerf-qt-tests -platform offscreen -o r.xml,xml        one report per class:
//                                                         r.DeviceTest.xml, r.ThemeTest.xml, ...
//
// QTest's own options pass through to each class, except a log written to a
// file. Each class is a QTest run of its own, and each run would rewrite the
// same file, leaving only the last class's results in it; so when more than
// one class runs, every `-o <name>,<format>` (or `-o <name>`) whose name is
// not `-` writes one file per class instead, with the class's name put before
// the file's extension: `-o out/r.xml,xml` gives out/r.DeviceTest.xml and so
// on. The run says which files it wrote. When one class runs, the file is the
// one named. The run fails when any class fails, and ends with the totals of
// all classes unless the caller asked for a log of their own.

#include "editor_common.h"
#include "editor_theme.h"
#include "test_registry.h"

#include <QApplication>
#include <QByteArray>
#include <QCoreApplication>
#include <QFile>
#include <QGuiApplication>
#include <QList>
#include <QMetaMethod>
#include <QMouseEvent>
#include <QProcess>
#include <QSet>
#include <QSettings>
#include <QStringList>
#include <QTemporaryDir>
#include <QTest>
#include <QTextStream>
#include <QWidget>

#include <algorithm>
#include <cstdio>
#include <memory>
#include <vector>

namespace kerf::test {

namespace {

struct TestClass {
    const QMetaObject *meta;
    TestFactory make;
};

std::vector<TestClass> &testClasses() {
    static std::vector<TestClass> classes;
    return classes;
}

// Runs one class only, by its name; the run passes it to itself for the
// listings QTest ends the process after.
constexpr char kOnlyClass[] = "--kerf-test-class";

// QTest's options that take a value as the next argument.
bool takesValue(const QByteArray &option) {
    static const QSet<QByteArray> options{"-o", "-eventdelay", "-keydelay", "-mousedelay", "-maxwarnings", "-repeat",
                                          "-minimumvalue", "-minimumtotal", "-iterations", "-median", "-seed",
                                          "-perfcounter", kOnlyClass};
    return options.contains(option);
}

// Whether `method` is one QTest runs as a test function: a private slot
// with no arguments that returns nothing and is not a set-up, clean-up or
// data function.
bool isTestFunction(const QMetaMethod &method) {
    if (method.access() != QMetaMethod::Private || method.methodType() != QMetaMethod::Slot ||
        method.parameterCount() != 0 || method.returnType() != QMetaType::Void)
        return false;
    const QByteArray name = method.name();
    return !(name.isEmpty() || name.endsWith("_data") || name == "initTestCase" || name == "cleanupTestCase" ||
             name == "init" || name == "cleanup");
}

bool hasTestFunction(const QMetaObject *meta, const QByteArray &name) {
    const int index = meta->indexOfMethod(QByteArray(name + "()").constData());
    return index >= meta->methodOffset() && isTestFunction(meta->method(index));
}

struct Counts {
    int passed = 0;
    int failed = 0;
    int skipped = 0;
};

// The results in a TAP log QTest wrote, without the initTestCase and
// cleanupTestCase QTest reports for every class.
Counts tapCounts(const QString &path) {
    Counts counts;
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text)) return counts;
    QTextStream in(&file);
    while (!in.atEnd()) {
        const QString line = in.readLine();
        const bool ok = line.startsWith(QStringLiteral("ok "));
        if (!ok && !line.startsWith(QStringLiteral("not ok "))) continue;
        if (line.contains(QStringLiteral(" - initTestCase()")) || line.contains(QStringLiteral(" - cleanupTestCase()")))
            continue;
        // An expected failure (# TODO) passes; an unexpected pass fails.
        const bool expected = line.contains(QStringLiteral("# TODO"));
        if (line.contains(QStringLiteral("# SKIP"))) ++counts.skipped;
        else if (ok != expected) ++counts.passed;
        else ++counts.failed;
    }
    return counts;
}

// A widget that takes the releases that let go of held buttons.
class ButtonSink final : public QWidget {
protected:
    void mouseReleaseEvent(QMouseEvent *event) override { event->accept(); }
};

// Releases every mouse button the application or QTest still holds. QTest
// keeps the buttons it pressed in a private-namespace variable, which a later
// Qt may change, so it is not read: the buttons a test presses (left, right,
// middle) are released whether held or not, which costs nothing when one is
// not, along with any other the application reports held.
void releaseHeldButtons() {
    const Qt::MouseButtons held =
        QGuiApplication::mouseButtons() | Qt::LeftButton | Qt::RightButton | Qt::MiddleButton;
    ButtonSink sink;
    for (unsigned bit = 1; bit != 0 && bit <= unsigned(Qt::MaxMouseButton); bit <<= 1)
        if (held.testFlag(Qt::MouseButton(bit))) QTest::mouseRelease(&sink, Qt::MouseButton(bit));
}

} // namespace

// The class's name without its namespace: kerf::DeviceTest is DeviceTest.
QByteArray shortName(const QMetaObject *meta) {
    const QByteArray name(meta->className());
    const qsizetype colon = name.lastIndexOf(':');
    return colon < 0 ? name : name.mid(colon + 1);
}

// `file` with `name` put before its extension: r.xml is r.<name>.xml, and r
// (or a file in a folder whose name has a dot) is r.<name>.
QByteArray fileFor(const QByteArray &file, const QByteArray &name) {
    const qsizetype slash = std::max(file.lastIndexOf('/'), file.lastIndexOf('\\'));
    const qsizetype dot = file.lastIndexOf('.');
    // A dot that starts the file's name (.log) is not an extension.
    if (dot <= slash + 1) return file + '.' + name;
    return file.left(dot) + '.' + name + file.mid(dot);
}

// The value of an `-o` (`<file>,<format>` or `<file>`), writing to a file of
// the class's own; standard output (`-`) stays as it is.
QByteArray logFor(const QByteArray &value, const QByteArray &name) {
    const qsizetype comma = value.lastIndexOf(',');
    const QByteArray file = comma < 0 ? value : value.left(comma);
    if (file.isEmpty() || file == "-") return value;
    return fileFor(file, name) + (comma < 0 ? QByteArray() : value.mid(comma));
}

// QTest's options for one class: each file log becomes the class's own when
// more than one class runs.
QList<QByteArray> optionsFor(const QList<QByteArray> &options, const QMetaObject *meta, bool several) {
    QList<QByteArray> out = options;
    if (!several) return out;
    for (qsizetype i = 0; i + 1 < out.size(); ++i)
        if (out[i] == "-o") {
            out[i + 1] = logFor(out[i + 1], shortName(meta));
            ++i;
        }
    return out;
}

bool registerTestClass(const QMetaObject *meta, TestFactory make) {
    testClasses().push_back({meta, make});
    return true;
}

} // namespace kerf::test

int main(int argc, char *argv[]) {
    using namespace kerf;
    using namespace kerf::test;

    // As QTEST_MAIN sets a widget test up.
    QApplication app(argc, argv);
    app.setAttribute(Qt::AA_Use96Dpi, true);
#ifdef QT_KEYPAD_NAVIGATION
    QApplication::setNavigationMode(Qt::NavigationModeNone);
#endif
    QTest::setMainSourcePath(__FILE__);

    // Preferences go to a file of this run's own, apart from the editor's.
    // Windows' registry keeps nothing for an application with no
    // organization, so the tests name one.
    QTemporaryDir settings;
    if (!settings.isValid()) {
        std::fprintf(stderr, "kerf-qt-tests: no temporary folder for the test settings\n");
        return 1;
    }
    QSettings::setDefaultFormat(QSettings::IniFormat);
    QSettings::setPath(QSettings::IniFormat, QSettings::UserScope, settings.path());
    QCoreApplication::setOrganizationName(QStringLiteral("Industrial Kerf Tests"));
    useEditorStyle();
    // Tests draw in the dark appearance unless they pick one: offscreen
    // reports a light system, and painted colours are checked in dark.
    setEditorColorScheme(ColorScheme::Dark);
    if (!editorTheme().dark) {
        std::fprintf(stderr, "kerf-qt-tests: the editor did not take the dark appearance\n");
        return 1;
    }

    std::vector<TestClass> classes = testClasses();
    std::sort(classes.begin(), classes.end(), [](const TestClass &a, const TestClass &b) {
        return qstrcmp(a.meta->className(), b.meta->className()) < 0;
    });

    // QTest's options, kept for every class, and the test functions asked for.
    QList<QByteArray> options;
    QList<QByteArray> functions;
    QByteArray onlyClass;
    bool listFunctions = false;
    bool listTags = false;
    bool help = false;
    bool ownLog = false;
    for (int i = 1; i < argc; ++i) {
        const QByteArray arg(argv[i]);
        if (arg == kOnlyClass) {
            if (i + 1 < argc) onlyClass = argv[++i];
            continue;
        }
        if (arg.startsWith('-')) {
            options << arg;
            listFunctions = listFunctions || arg == "-functions";
            listTags = listTags || arg == "-datatags";
            help = help || arg == "-help" || arg == "--help" || arg == "-h";
            // A log of the caller's own, or a run only the plain text logger
            // supports, leaves the totals out.
            static const QSet<QByteArray> logging{"-o", "-txt", "-csv", "-junitxml", "-xml", "-lightxml", "-teamcity",
                                                  "-tap", "-repeat"};
            ownLog = ownLog || logging.contains(arg);
            if (takesValue(arg) && i + 1 < argc) options << QByteArray(argv[++i]);
        } else {
            functions << arg;
        }
    }
    if (!onlyClass.isEmpty())
        classes.erase(std::remove_if(classes.begin(), classes.end(),
                                     [&](const TestClass &test) { return onlyClass != test.meta->className(); }),
                      classes.end());

    // QTest prints the test functions and then ends the process, so the
    // union of every class's is printed here.
    if (listFunctions && !help) {
        for (const TestClass &test : classes)
            for (int index = test.meta->methodOffset(); index < test.meta->methodCount(); ++index)
                if (isTestFunction(test.meta->method(index)))
                    std::printf("%s\n", test.meta->method(index).methodSignature().constData());
        return 0;
    }
    // Each function asked for, by the class it belongs to.
    QList<QList<QByteArray>> picked(qsizetype(classes.size()));
    for (const QByteArray &function : functions) {
        const qsizetype colon = function.indexOf(':');
        const QByteArray name = colon < 0 ? function : function.left(colon);
        bool found = false;
        for (size_t c = 0; c < classes.size() && !found; ++c)
            if (hasTestFunction(classes[c].meta, name)) {
                picked[qsizetype(c)] << function;
                found = true;
            }
        if (!found) {
            std::fprintf(stderr, "kerf-qt-tests: no test class has a test function named %s (see -functions)\n",
                         name.constData());
            return 1;
        }
    }

    const auto runs = [&](size_t c) { return functions.isEmpty() || !picked[qsizetype(c)].isEmpty(); };
    int toRun = 0;
    for (size_t c = 0; c < classes.size(); ++c) toRun += runs(c) ? 1 : 0;
    const bool several = toRun > 1;
    if (several && !help)
        for (qsizetype i = 0; i + 1 < options.size(); ++i)
            if (options[i] == "-o" && logFor(options[i + 1], "Class") != options[i + 1]) {
                const QByteArray &value = options[i + 1];
                const qsizetype comma = value.lastIndexOf(',');
                const QByteArray file = comma < 0 ? value : value.left(comma);
                std::fprintf(stderr, "kerf-qt-tests: -o %s writes one file per test class, such as %s\n",
                             value.constData(), fileFor(file, QByteArray("<Class>")).constData());
                ++i;
            }

    // The data tags QTest also ends the process after: one run of this
    // program for each class.
    if (listTags && !help && onlyClass.isEmpty()) {
        int failed = 0;
        for (size_t c = 0; c < classes.size(); ++c) {
            if (!runs(c)) continue;
            const TestClass &test = classes[c];
            QStringList args;
            for (const QByteArray &option : optionsFor(options, test.meta, several))
                args << QString::fromLocal8Bit(option);
            for (const QByteArray &function : picked[qsizetype(c)]) args << QString::fromLocal8Bit(function);
            QProcess child;
            child.setProcessChannelMode(QProcess::ForwardedChannels);
            child.start(QCoreApplication::applicationFilePath(),
                        args << QString::fromLatin1(kOnlyClass) << QString::fromLatin1(test.meta->className()));
            if (!child.waitForFinished(-1) || child.exitStatus() != QProcess::NormalExit || child.exitCode() != 0) ++failed;
        }
        return failed > 0 ? 1 : 0;
    }

    const bool counting = !help && !ownLog;
    QTemporaryDir logs;
    Counts total;
    int failedClasses = 0;
    int ran = 0;
    for (size_t c = 0; c < classes.size(); ++c) {
        if (!runs(c)) continue;
        const QString tap = logs.filePath(QString::fromLatin1(classes[c].meta->className()).replace(QStringLiteral("::"), QStringLiteral("_")) +
                                          QStringLiteral(".tap"));
        QList<QByteArray> args{QByteArray(argv[0])};
        args << optionsFor(options, classes[c].meta, several);
        if (counting && logs.isValid()) args << "-o" << "-,txt" << "-o" << QFile::encodeName(tap) + ",tap";
        args << picked[qsizetype(c)];
        std::vector<char *> raw;
        for (QByteArray &arg : args) raw.push_back(arg.data());
        raw.push_back(nullptr);

        // Each class starts with no mouse button held, as it would alone: a
        // QTest::mouseDClick on a widget presses one without releasing it,
        // which would turn a later class's mouse moves into drags.
        releaseHeldButtons();
        const std::unique_ptr<QObject> test = classes[c].make();
        const int failures = QTest::qExec(test.get(), int(raw.size()) - 1, raw.data());
        ++ran;
        if (failures != 0) ++failedClasses;
        if (counting && logs.isValid()) {
            const Counts counts = tapCounts(tap);
            total.passed += counts.passed;
            total.failed += counts.failed;
            total.skipped += counts.skipped;
        }
        if (help) break;
    }

    if (counting)
        std::printf("Totals of %d test classes: %d passed, %d failed, %d skipped (initTestCase and cleanupTestCase not counted)\n",
                    ran, total.passed, total.failed, total.skipped);
    if (failedClasses > 0) std::printf("%d of %d test classes failed\n", failedClasses, ran);
    return failedClasses > 0 ? std::min(failedClasses, 127) : 0;
}
